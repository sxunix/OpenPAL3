//! 野外怪物的 AI（`FUN_00417420`，角色行为 5 = 追击）。逆向记在 `switch/re/notes/combat_flow.md`
//! 「野外遇敌」一节。这里管决策、受力和找路，位置怎么挪由 [`super::ScnScene`] 做，落点判定在
//! [`FieldMonster::settle`]。
//!
//! 原版一帧的顺序：`FUN_00413cd0` 按移动方式（`+0x4f4`：0 不动、1 晃、2 走路径）算受力 → 状态机
//! （`+0x588`：0 待命、1 追、2 回、3 定住）→ 受力积进速度（`+0x510 += +0x528 × dt`，y 清零，截到最高速
//! `+0x508`）→ `FUN_00411e00` 挪位置（一步最多 18）。
//!
//! 原版好几处距离把角色的绝对 y 也算了进去（路径点的 y 是 0），场景地面高的时候效果会变：沿 A* 路径走时
//! 期望速度大半朝下、水平方向慢很多；「走到目标 12 以内」永远不成立。这些都照抄。

use radiance::math::Vec3;

use super::nav_path::{self, FieldNav, Path};
use crate::openpal3::combat::encounter::CrtRand;
use crate::openpal3::states::role_move::{MonsterConstants, RoleConstants};

/// gdb 怪物记录里的三个档位（`+0x2f1` / `+0x2f3` / `+0x2f4`）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Levels {
    /// 活动范围：0 不动、1 小、2 中、3 大。
    pub move_range: u8,
    /// 平时的速度：0 不动、1..3。
    pub move_speed: u8,
    /// 追击速度：0..2。
    pub chase_speed: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle,
    Chase,
    Return,
}

#[derive(Debug, Clone)]
pub enum Motion {
    /// `+0x4f4 = 0`：不受力、速度不积。
    Still,
    /// `+0x4f4 = 1`：随机晃（`FUN_00413cd0` case 1）。
    Wander,
    /// `+0x4f4 = 2`：沿路径走（`FUN_00413850`）。
    Path(Box<Path>),
}

/// 一只野外怪物。
#[derive(Debug, Clone)]
pub struct FieldMonster {
    pub role_index: u8,
    /// 出生点（角色 `+0xf8` / `+0xfc`，`.scn` 的位置）。
    pub home: (f32, f32),
    pub state: State,
    pub motion: Motion,
    /// 晃的方向（`+0x560`）。
    pub wander: Vec3,
    /// 速度（`+0x510`）。
    pub velocity: Vec3,
    force: Vec3,
    /// `+0x508` / `+0x50c`。
    pub max_speed: f32,
    pub max_force: f32,
    /// `+0x590` = FreeRange[活动范围]。
    pub free_range: f32,
    /// `+0x594` = AttackRange × FreeRange：追出这么远就回去。
    pub leash: f32,
    /// `+0x598` = FindRadiusL（写死 L 档）。
    pub find_radius: f32,
    pub levels: Levels,
    /// 找路的目标（`+0x540`）和它是不是个角色（`+0x5b4`：追主角时是，回家时不是）。
    target: Vec3,
    target_is_entity: bool,
    /// 看不见目标时隔多少帧重跑一次 A*（`+0x5b8`）。
    repath: u32,
}

/// 离怪物这么近直接发现，不看视野（`FUN_004178a0`）。
pub const SENSE_DISTANCE: f32 = 120.;
/// 沿路径往前看这么远（`FUN_00413850`）。
const LOOK_AHEAD: f32 = 36.;
/// 看不见目标时，超过这么多帧才重跑 A*（`FUN_00412e60`：`+0x5b8 > 0x1e`）。
const REPATH_FRAMES: u32 = 30;

fn dot(a: &Vec3, b: &Vec3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

impl FieldMonster {
    /// `FUN_00417040`：从 `.scn` 记录和 gdb 第一个怪物的档位建起来。活动范围档不是 0 的开场就晃
    /// （`FUN_00412a50`），是 0 的站着。
    pub fn new(
        role_index: u8,
        home: (f32, f32),
        facing: Vec3,
        levels: Levels,
        monster: &MonsterConstants,
        role: &RoleConstants,
    ) -> Self {
        // 0x417368 的跳转表：0、1 → S，2 → M，3 → L，其它 → 0。
        let free_range = match levels.move_range {
            0 | 1 => monster.free_range[0],
            2 => monster.free_range[1],
            3 => monster.free_range[2],
            _ => 0.,
        };
        let mut m = Self {
            role_index,
            home,
            state: State::Idle,
            motion: Motion::Still,
            wander: facing,
            velocity: Vec3::new(0., 0., 0.),
            force: Vec3::new(0., 0., 0.),
            max_speed: 0.,
            max_force: 0.,
            free_range,
            leash: monster.attack_range * free_range,
            find_radius: monster.find_radius[2],
            levels,
            target: Vec3::new(0., 0., 0.),
            target_is_entity: false,
            repath: 0,
        };
        if levels.move_range != 0 {
            m.start_wander(monster, role);
        }
        m
    }

    /// `FUN_00412710` 里怪物（`+0xad == 2`）走路那一支：追的时候按追击档取 AttackSpeed（1 → M、2 → L、
    /// 其它 → S），不追按速度档取 FreeSpeed（0 → 0、1 → S、2 → M、其它 → L）；推力都是 WalkForce。
    fn set_speed(&mut self, monster: &MonsterConstants, role: &RoleConstants) {
        self.max_speed = if self.state == State::Chase {
            match self.levels.chase_speed {
                1 => monster.attack_speed[1],
                2 => monster.attack_speed[2],
                _ => monster.attack_speed[0],
            }
        } else {
            match self.levels.move_speed {
                0 => 0.,
                1 => monster.free_speed[0],
                2 => monster.free_speed[1],
                _ => monster.free_speed[2],
            }
        };
        self.max_force = role.walk_force;
    }

    /// `FUN_00412a50`：开始晃，速度和推力都减半。
    fn start_wander(&mut self, monster: &MonsterConstants, role: &RoleConstants) {
        self.motion = Motion::Wander;
        self.set_speed(monster, role);
        self.max_speed *= 0.5;
        self.max_force *= 0.5;
    }

    /// `FUN_004142e0`：停下——移动方式归 0、速度清零。落点走不通时也是它（`FUN_00414350` 末尾），
    /// 所以晃着撞到边的怪就站住不晃了，直到发现主角；回家路上走到头或撞到边都当到了家。
    /// 受力（`+0x528`）不清：下一帧重新找到路时，积进速度的还是上一次算的受力。
    pub fn stop(&mut self) {
        self.motion = Motion::Still;
        self.velocity = Vec3::new(0., 0., 0.);
    }

    /// `FUN_004178a0`：看见主角没有。主角得在同一层、离出生点不超过 `+0x594 × 0.8`（原版这个距离
    /// 把主角的绝对 y 也算进去了）；离怪不到 120 直接看见，不到 `+0x598` 且在怪正前方（点积 > 0.7）、
    /// 导航网格上看得见（`FUN_004215b0`）也算。
    pub fn detects<N: FieldNav + ?Sized>(
        &self,
        pos: &Vec3,
        facing: &Vec3,
        player: &Vec3,
        same_layer: bool,
        nav: &N,
    ) -> bool {
        if !same_layer {
            return false;
        }
        let (hx, hz) = (player.x - self.home.0, player.z - self.home.1);
        if (hx * hx + player.y * player.y + hz * hz).sqrt() > self.leash * 0.8 {
            return false;
        }
        let d = Vec3::sub(player, pos);
        let dist = d.norm();
        if dist < SENSE_DISTANCE {
            return true;
        }
        if dist < self.find_radius {
            return dot(&d, facing) / dist > 0.7
                && nav_path::line_clear(nav, nav.tile_of(pos), nav.tile_of(player));
        }
        false
    }

    /// `FUN_00412d90`：A* 找路去 `target`。离目标（带怪的 y）不到 12 就什么都不做；找不到路就停下。
    /// 速度方式一直是 1，不重算速度。
    fn path_to<N: FieldNav + ?Sized>(&mut self, target: &Vec3, pos: &Vec3, nav: &N) -> bool {
        let (dx, dz) = (target.x - pos.x, target.z - pos.z);
        if pos.y * pos.y + dx * dx + dz * dz < 144. {
            return true;
        }
        match nav_path::find_path(nav, pos, target) {
            Some(p) => {
                self.motion = Motion::Path(Box::new(p));
                true
            }
            None => {
                self.stop();
                false
            }
        }
    }

    /// `FUN_00413010`：目标在同一层就 [`Self::path_to`]。[未做] 不在同一层时原版找两层之间的通道口走过去。
    fn retarget<N: FieldNav + ?Sized>(
        &mut self,
        target: &Vec3,
        pos: &Vec3,
        same_layer: bool,
        nav: &N,
    ) {
        if same_layer {
            self.path_to(target, pos, nav);
        } else {
            self.stop();
        }
    }

    /// `FUN_00412e60`：朝一个角色找路。还没在走路径就直接找；在走路径、同一层、看得见，路径就换成
    /// 「现在的位置 → 目标」一条直线；看不见就每隔 30 多帧重跑一次 A*（`urgent` 时马上跑）。
    fn chase_path<N: FieldNav + ?Sized>(
        &mut self,
        target: &Vec3,
        pos: &Vec3,
        same_layer: bool,
        urgent: bool,
        nav: &N,
    ) {
        self.target = *target;
        self.target_is_entity = true;
        if !matches!(self.motion, Motion::Path(_)) {
            self.retarget(target, pos, same_layer, nav);
            self.repath = 0;
            return;
        }
        if same_layer && nav_path::line_clear(nav, nav.tile_of(pos), nav.tile_of(target)) {
            self.motion = Motion::Path(Box::new(Path::new(vec![*pos, *target])));
            return;
        }
        if !urgent {
            self.repath += 1;
            if self.repath > REPATH_FRAMES {
                self.repath = 0;
                self.retarget(target, pos, same_layer, nav);
            }
            return;
        }
        self.repath = 0;
        self.retarget(target, pos, same_layer, nav);
    }

    /// `FUN_00413850`：沿路径走一帧。返回 `true` 表示到了（调用方停下）；到路径末端时顺带要求把位置
    /// 直接放到终点（第二个返回值）。
    fn follow_path<N: FieldNav + ?Sized>(
        &mut self,
        pos: &Vec3,
        facing: &Vec3,
        same_layer: bool,
        nav: &N,
    ) -> (bool, Option<Vec3>) {
        if self.target_is_entity {
            let (dx, dz) = (self.target.x - pos.x, self.target.z - pos.z);
            if pos.y * pos.y + dx * dx + dz * dz < 144. {
                return (true, None);
            }
        }
        let Motion::Path(path) = &self.motion else {
            return (false, None);
        };
        // 往前看 36：那一点就在路上（6 以内）、那一段又跟朝向差不多（点积 > 0.5），就照直走。
        let ahead = Vec3::add(pos, &Vec3::scalar_mul(LOOK_AHEAD, facing));
        let (on, dir) = path.near(&ahead);
        let heading = if on && dot(&dir, facing) > 0.5 {
            *facing
        } else {
            let walked = path.project(pos);
            let point = if path.total <= walked + LOOK_AHEAD {
                if self.target_is_entity {
                    let target = self.target;
                    self.chase_path(&target, pos, same_layer, true, nav);
                    return (false, None);
                }
                let n = path.points.len();
                let (last, before) = (path.points[n - 1], path.points[n - 2]);
                let seg = Vec3::sub(&last, &before);
                let l = seg.norm();
                let along = dot(&seg, &Vec3::sub(pos, &before)) / l;
                if l < 5e-5 || (along >= 0. && along - l > -0.1) {
                    return (true, Some(last));
                }
                last
            } else {
                path.point_at(walked + LOOK_AHEAD)
                    .unwrap_or(Vec3::new(0., 0., 0.))
            };
            Vec3::normalized(&Vec3::sub(&point, pos))
        };
        // `FUN_00414250`：推力 = 期望速度 − 现在的速度，截到最大推力。
        let desired = Vec3::scalar_mul(self.max_speed, &heading);
        self.force = clamp(Vec3::sub(&desired, &self.velocity), self.max_force);
        (false, None)
    }

    /// `FUN_00414350`：这一步能不能落在 `to`。那一格走得通就落；走不通而正沿路径走时，把 `to` 投到
    /// 路径上（y 取 0），往前（跟朝向点积 > 0）而且那一格走得通就落在那儿；都不行就停下。原版还有一支
    /// 试周围八格滑过去，只给行为 3 的角色用，怪是行为 5。
    pub fn settle<N: FieldNav + ?Sized>(
        &mut self,
        pos: &Vec3,
        to: &Vec3,
        facing: &Vec3,
        nav: &N,
    ) -> Option<Vec3> {
        if nav.walkable_at(to) {
            return Some(*to);
        }
        if let Motion::Path(path) = &self.motion {
            let mut c = path.point_at(path.project(to)).unwrap_or(*to);
            c.y = 0.;
            let d = Vec3::sub(&c, pos);
            // 原版这里还要求落点离 `to` 不到 12，可比的两个指针指着同一块内存，永远成立。
            if d.norm() > 0.05 && dot(&Vec3::normalized(&d), facing) > 0. && nav.walkable_at(&c) {
                return Some(c);
            }
        }
        self.stop();
        None
    }

    /// 追主角（`FUN_00417850`）：朝主角找路，速度按追的算。
    fn begin_chase<N: FieldNav + ?Sized>(
        &mut self,
        player: &Vec3,
        pos: &Vec3,
        same_layer: bool,
        nav: &N,
        monster: &MonsterConstants,
        role: &RoleConstants,
    ) {
        self.state = State::Chase;
        self.chase_path(player, pos, same_layer, true, nav);
        self.set_speed(monster, role);
    }

    /// 一帧的决策、受力和找路，返回积好的速度；到路径末端被直接放到终点时第二个值是新位置。`pos` /
    /// `facing` 是现在的位置和朝向，`player` 是主角位置（不在同一层就 `same_layer = false`）。
    #[allow(clippy::too_many_arguments)]
    pub fn step<N: FieldNav + ?Sized>(
        &mut self,
        dt: f32,
        pos: &Vec3,
        facing: &Vec3,
        player: &Vec3,
        same_layer: bool,
        rng: &mut CrtRand,
        nav: &N,
        monster: &MonsterConstants,
        role: &RoleConstants,
    ) -> (Vec3, Option<Vec3>) {
        let mut here = *pos;
        let mut placed = None;
        // `FUN_00413cd0`：按移动方式算受力。
        match self.motion {
            Motion::Still => {}
            Motion::Wander => self.wander_force(facing, rng),
            Motion::Path(_) => {
                let (arrived, end) = self.follow_path(pos, facing, same_layer, nav);
                if arrived {
                    if let Some(end) = end {
                        placed = self.settle(pos, &end, facing, nav);
                        if let Some(p) = placed {
                            here = p;
                        }
                    }
                    self.stop();
                }
            }
        }

        match self.state {
            State::Idle => {
                if self.detects(&here, facing, player, same_layer, nav) {
                    self.begin_chase(player, &here, same_layer, nav, monster, role);
                } else {
                    self.pull_back(&here);
                }
            }
            State::Chase => {
                self.chase_path(player, &here, same_layer, false, nav);
                // 离出生点超过 `+0x594` 就回（这个距离带着怪自己的绝对 y）。
                let (hx, hz) = (here.x - self.home.0, here.z - self.home.1);
                if (hx * hx + here.y * here.y + hz * hz).sqrt() > self.leash {
                    self.state = State::Return;
                    self.target_is_entity = false;
                    let home = Vec3::new(self.home.0, 0., self.home.1);
                    self.path_to(&home, &here, nav);
                }
            }
            State::Return => {
                if matches!(self.motion, Motion::Still) {
                    // 到家：待命、开始晃（不管活动范围档是不是 0），晃的方向从现在的朝向起。
                    self.state = State::Idle;
                    self.start_wander(monster, role);
                    self.wander = *facing;
                } else if self.detects(&here, facing, player, same_layer, nav) {
                    self.begin_chase(player, &here, same_layer, nav, monster, role);
                }
            }
        }

        // `FUN_00417420` 末尾：受力积进速度，y 清零，截到最高速。
        if !matches!(self.motion, Motion::Still) {
            self.velocity = Vec3::add(&self.velocity, &Vec3::scalar_mul(dt, &self.force));
            self.velocity.y = 0.;
            self.velocity = clamp(self.velocity, self.max_speed);
        }
        (self.velocity, placed)
    }

    /// `FUN_00413cd0` case 1：晃。三个 `rand()` 拼一个随机方向加 0.2 倍进晃的方向，再按
    /// 右 / 上 / 前三根轴展开乘 `0.2 × 最大推力`，加上往前的 `0.2 × 1.414 × 最大推力`，截到 `0.3 × 最大推力`。
    fn wander_force(&mut self, facing: &Vec3, rng: &mut CrtRand) {
        let f = self.max_force * 0.2;
        let (r1, r2, r3) = (rng.rand(), rng.rand(), rng.rand());
        let r = Vec3::normalized(&Vec3::new(
            r3 as f32 * 6.103_702e-5 - 1.,
            r2 as f32 * 6.103_702e-5 - 1.,
            r1 as f32 * 6.103_702e-5 - 1.,
        ));
        self.wander.x += r.x * 0.2;
        self.wander.z += r.z * 0.2;
        self.wander = Vec3::normalized(&self.wander);
        // 右 = 前 × 上（`FUN_004122c0`，上是 (0,1,0)）。
        let right = Vec3::new(-facing.z, 0., facing.x);
        let steer = Vec3::new(
            (right.x * self.wander.x + facing.x * self.wander.z) * f,
            self.wander.y * f,
            (right.z * self.wander.x + facing.z * self.wander.z) * f,
        );
        let ahead = Vec3::scalar_mul(f * 1.414, facing);
        let mut force = clamp(Vec3::add(&ahead, &steer), self.max_force * 0.3);
        force.y = 0.;
        self.force = force;
    }

    /// `FUN_0041ad40`：照现在的速度 1.4 秒后会出活动范围，就加一把往出生点的力（超出多少 × 1.4），
    /// 总受力截到最大推力。原版拿晃的方向当临时变量，算完它就指着出生点了——连怪的 y 一起归一化，
    /// 地面高的场景里这把力大半朝下、拉回来得很慢。
    fn pull_back(&mut self, pos: &Vec3) {
        let (px, pz) = (
            self.velocity.x * 1.4 + pos.x - self.home.0,
            self.velocity.z * 1.4 + pos.z - self.home.1,
        );
        let out = (px * px + pz * pz).sqrt();
        if out <= self.free_range {
            return;
        }
        self.wander =
            Vec3::normalized(&Vec3::new(self.home.0 - pos.x, -pos.y, self.home.1 - pos.z));
        let k = (out - self.free_range) * 1.4;
        self.force = clamp(
            Vec3::add(&self.force, &Vec3::scalar_mul(k, &self.wander)),
            self.max_force,
        );
    }
}

fn clamp(v: Vec3, max: f32) -> Vec3 {
    let n = v.norm();
    if n > max && n > 0. {
        Vec3::scalar_mul(max / n, &v)
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openpal3::scene::nav_path::TestGrid;

    const HOME: (f32, f32) = (1000., 1000.);

    fn consts() -> (MonsterConstants, RoleConstants) {
        (MonsterConstants::default(), RoleConstants::default())
    }

    fn monster(levels: Levels) -> FieldMonster {
        let (m, r) = consts();
        FieldMonster::new(1, HOME, Vec3::new(0., 0., 1.), levels, &m, &r)
    }

    fn at(dx: f32, dz: f32) -> Vec3 {
        Vec3::new(HOME.0 + dx, 0., HOME.1 + dz)
    }

    #[test]
    fn ranges_and_speeds_follow_the_level_tables() {
        let m = monster(Levels {
            move_range: 2,
            move_speed: 1,
            chase_speed: 2,
        });
        assert_eq!(m.free_range, 60.);
        assert_eq!(m.leash, 600.);
        assert_eq!(m.find_radius, 400.);
        assert!(matches!(m.motion, Motion::Wander));
        assert_eq!(
            (m.max_speed, m.max_force),
            (15., 400.),
            "晃的时候 FreeSpeedS、WalkForce 各减半"
        );

        let still = monster(Levels::default());
        assert!(
            matches!(still.motion, Motion::Still),
            "活动范围档 0 开场不晃"
        );
        assert_eq!(still.free_range, 40.);
    }

    #[test]
    fn it_sees_the_player_close_by_or_in_front_and_in_sight() {
        let m = monster(Levels {
            move_range: 3,
            ..Default::default()
        });
        let mut g = TestGrid::open(200, 200, 10.);
        let ahead = Vec3::new(0., 0., 1.);
        assert!(m.detects(&at(0., 0.), &ahead, &at(0., -100.), true, &g));
        assert!(m.detects(&at(0., 0.), &ahead, &at(0., 300.), true, &g));
        assert!(
            !m.detects(&at(0., 0.), &ahead, &at(0., -300.), true, &g),
            "在背后"
        );
        assert!(
            !m.detects(&at(0., 0.), &ahead, &at(0., 50.), false, &g),
            "不同层"
        );
        g.walls.insert((100, 115));
        assert!(
            !m.detects(&at(0., 0.), &ahead, &at(0., 300.), true, &g),
            "挡住了"
        );
    }

    #[test]
    fn it_chases_at_attack_speed_and_gives_up_past_the_leash() {
        let (mc, rc) = consts();
        let g = TestGrid::open(200, 200, 10.);
        let mut m = monster(Levels {
            move_range: 1,
            move_speed: 1,
            chase_speed: 1,
        });
        let mut rng = CrtRand::new(3);
        let fwd = Vec3::new(0., 0., 1.);
        let player = at(0., 50.);
        m.step(
            0.1,
            &at(0., 0.),
            &fwd,
            &player,
            true,
            &mut rng,
            &g,
            &mc,
            &rc,
        );
        assert_eq!(m.state, State::Chase);
        assert!(matches!(m.motion, Motion::Path(_)));
        assert_eq!(m.max_speed, 210., "追击档 1 → AttackSpeedM");
        let far = at(0., m.leash + 1.);
        m.step(0.1, &far, &fwd, &player, true, &mut rng, &g, &mc, &rc);
        assert_eq!(m.state, State::Return);
        let Motion::Path(p) = &m.motion else {
            panic!("回家要找路");
        };
        let end = p.points.last().unwrap();
        assert_eq!((end.x, end.z), (1005., 1005.), "终点是出生点那一格的中心");
    }

    #[test]
    fn a_chase_routes_around_a_wall_and_walks_home_afterwards() {
        let (mc, rc) = consts();
        let mut g = TestGrid::open(200, 200, 10.);
        for x in 95..106 {
            g.walls.insert((x, 105));
        }
        let mut m = monster(Levels {
            move_range: 1,
            move_speed: 1,
            chase_speed: 2,
        });
        let mut rng = CrtRand::new(5);
        let mut pos = at(0., 0.);
        let mut facing = Vec3::new(0., 0., 1.);
        let player = at(0., 90.);
        let mut reached = false;
        for _ in 0..400 {
            let (v, placed) = m.step(
                1. / 30.,
                &pos,
                &facing,
                &player,
                true,
                &mut rng,
                &g,
                &mc,
                &rc,
            );
            if let Some(p) = placed {
                pos = p;
            }
            if v.norm() > 0.01 {
                let step = Vec3::scalar_mul(1. / 30., &v);
                let to = Vec3::add(&pos, &step);
                if let Some(p) = m.settle(&pos, &to, &facing, &g) {
                    pos = p;
                    facing = Vec3::normalized(&v);
                }
            }
            assert!(!g.walls.contains(&g.tile_of(&pos)), "穿墙了：{pos:?}");
            if Vec3::sub(&pos, &player).norm() < 30. {
                reached = true;
                break;
            }
        }
        assert_eq!(m.state, State::Chase);
        assert!(reached, "绕过墙追到了主角附近：{pos:?}");
    }

    #[test]
    fn wandering_force_stays_under_three_tenths_of_the_cap() {
        let (mc, rc) = consts();
        let g = TestGrid::open(200, 200, 10.);
        let mut m = monster(Levels {
            move_range: 1,
            move_speed: 2,
            chase_speed: 0,
        });
        let mut rng = CrtRand::new(9);
        let fwd = Vec3::new(1., 0., 0.);
        let away = at(900., 900.);
        for _ in 0..50 {
            m.step(0.05, &at(0., 0.), &fwd, &away, true, &mut rng, &g, &mc, &rc);
            assert!(m.force.norm() <= m.max_force * 0.3 + 1e-3);
            assert!(m.velocity.norm() <= m.max_speed + 1e-3);
        }
    }
}
