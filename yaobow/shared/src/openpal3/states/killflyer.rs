//! 打苍蝇（SCE 105）的规则。逆向记录在 `switch/re/notes/killflyer.md`。
//!
//! 一局就是一波：掷一次骰决定出几只（10% 出三只、第一只是金的），玩家在
//! 场地里拍；拍中的往下掉，**落到底才算钱**；连着两次越界的飞走，不给钱。
//! 三只都结束这一波就完，钱加进身上、花费从景天的精里扣。

/// 场地（原版就是整屏 800×600，构造函数里写死的 rect）。
pub const FIELD_W: f32 = 800.;
pub const FIELD_H: f32 = 600.;
/// 苍蝇的尺寸（`+0x6ac = 0x1c`）；落地线是 `FIELD_H − 2 × FLY_SIZE`。
pub const FLY_SIZE: f32 = 28.;
/// 出苍蝇的概率（`+0x71c`，构造函数给 1.0——每次必出）。
pub const SPAWN_CHANCE: f32 = 1.0;
/// 出金苍蝇的概率（`+0x724`，掷的是 `rand10 < 102.4`）。
pub const GOLD_CHANCE: f32 = 0.1;
/// 出金苍蝇那一波是三只。
pub const GOLD_WAVE_FLIES: usize = 3;
/// 拍中之后的下落初速（`0x3e99999a`）与每帧加速度（`0x56aa64`）。
pub const FALL_SPEED0: f32 = 0.3;
pub const FALL_ACCEL: f32 = 0.01;
/// 越界几次就飞走（`+0x6a0[i]` 加到 2）。
pub const EDGE_LIMIT: u32 = 2;

/// 一只苍蝇的状态（原版 `+0x64c + i*4`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlyState {
    /// 0：在飞。
    Flying,
    /// 1：被拍中，正在掉。
    Falling,
    /// 2：飞走了，不给钱。
    Gone,
    /// 3：这一只结束了。
    Done,
}

/// 场上的一只苍蝇。
#[derive(Debug, Clone)]
pub struct Fly {
    pub x: f32,
    pub y: f32,
    /// 现在飞向哪儿。
    pub target: (f32, f32),
    /// 速度系数 `(rand15() % 12 − 6) × 0.1 + 0.8`，0.2–1.3。
    pub speed: f32,
    /// 拍中能拿多少钱：金的 100–199，普通的 1–20。
    pub value: i32,
    pub gold: bool,
    pub state: FlyState,
    /// 掉落速度。
    pub fall: f32,
    /// 越了几次界。
    pub edges: u32,
}

impl Fly {
    /// 鼠标点在不在这只身上。
    pub fn hit(&self, x: f32, y: f32) -> bool {
        self.state == FlyState::Flying
            && (x - self.x).abs() <= FLY_SIZE * 0.5
            && (y - self.y).abs() <= FLY_SIZE * 0.5
    }
}

/// 原版的随机数（和店铺那边同一套 MSVC `rand`）。
#[derive(Debug, Clone)]
pub struct FlyRng {
    seed: u32,
}

impl FlyRng {
    pub fn new(seed: u32) -> Self {
        Self { seed }
    }

    pub fn rand15(&mut self) -> i32 {
        self.seed = self.seed.wrapping_mul(0x343F_D).wrapping_add(0x269E_C3);
        (self.seed >> 16 & 0x7FFF) as i32
    }

    pub fn rand10(&mut self) -> i32 {
        self.seed = self.seed.wrapping_mul(0x343F_D).wrapping_add(0x269E_C3);
        (self.seed >> 16 & 0x3FF) as i32
    }

    /// 和逃跑、还价同一个掷法：`rand10 < round(概率 × 100 × 10.24)`。
    pub fn chance(&mut self, p: f32) -> bool {
        let n = (p * 100.) as i32;
        if n <= 0 {
            return false;
        }
        self.rand10() < (n as f32 * 10.24).round() as i32
    }
}

/// 一局。
#[derive(Debug, Clone)]
pub struct FlyGame {
    pub flies: Vec<Fly>,
    /// 这一局打出来的钱（原版 `+0x65c`）。
    pub earned: i32,
    /// 打死了几只（`+0x660`）。
    pub killed: u32,
    rng: FlyRng,
}

impl FlyGame {
    /// 掷骰开一波。出不出、出几只都照 `FUN_0052aab0` 的两次掷骰。
    pub fn new(seed: u32) -> Self {
        let mut rng = FlyRng::new(seed);
        let mut flies = Vec::new();
        if rng.chance(SPAWN_CHANCE) {
            let count = if rng.chance(GOLD_CHANCE) {
                GOLD_WAVE_FLIES
            } else {
                1
            };
            for i in 0..count {
                // 三只那一波，第一只是金的（值 100–199），其余普通（1–20）。
                let gold = count == GOLD_WAVE_FLIES && i == 0;
                let value = if gold {
                    rng.rand15() % 100 + 100
                } else {
                    rng.rand15() % 20 + 1
                };
                let speed = (rng.rand15() % 12 - 6) as f32 * 0.1 + 0.8;
                let x = (rng.rand15() % FIELD_W as i32) as f32;
                let y = (rng.rand15() % FIELD_H as i32) as f32;
                flies.push(Fly {
                    x,
                    y,
                    target: (x, y),
                    speed,
                    value,
                    gold,
                    state: FlyState::Flying,
                    fall: 0.,
                    edges: 0,
                });
            }
        }
        Self {
            flies,
            earned: 0,
            killed: 0,
            rng,
        }
    }

    /// 拍一下。中了就让那只开始往下掉（钱要等它落地才算）。
    pub fn swat(&mut self, x: f32, y: f32) -> bool {
        for fly in self.flies.iter_mut() {
            if fly.hit(x, y) {
                fly.state = FlyState::Falling;
                fly.fall = FALL_SPEED0;
                return true;
            }
        }
        false
    }

    /// 走一帧。
    ///
    /// 飞行那一支照 `KF_Sys::Update`（`subgame/KillFlyer/KF_Sys.cpp:296` 起，
    /// exe 里 `FUN_00529e40`）：**两根轴各走各的**，不是沿着方向向量走一步——
    ///
    /// ```cpp
    /// if ( m_pt[i].x < m_nextpt[i].x )      m_pt[i].x += 速度;
    /// else if ( m_pt[i].x > m_nextpt[i].x ) m_pt[i].x -= 速度;
    /// ... 越界检查 ... 到了就 m_isGetNextpt = true; break;
    /// // y 同形
    /// ```
    ///
    /// 所以斜着飞比直着快 √2 倍；哪根轴先到目标就立刻换一个新目标点，
    /// 另一根轴到没到都不管（`break` 跳出整段）。目标点在整块
    /// 800 × 600 区域里随机取。
    ///
    /// 越界：位置超过 `边 ∓ m_flyoutparam`（`m_flyoutparam = 28`）就
    /// 计数加一、换目标点，加到 2 就飞走。原版**不把位置夹回来**。
    ///
    /// 速度 `m_speed[i] = ((rand()%12) - 6) * 0.1f + 0.8f`，也就是
    /// 0.2 到 1.3（源码旁边那行「0.5~~1.1」的注释是错的）；
    /// `m_specspeed[i] = m_speed[i] * 1.6f` 给 `m_type[i] != 0` 的那种用，
    /// 移植端还没分这两种。
    ///
    /// 这些量都是**按帧**加的，原版没有帧率上限，这里和别处一样按
    /// 每秒 60 帧折算。
    pub fn tick(&mut self, delta_sec: f32) {
        let floor = FIELD_H - 2. * FLY_SIZE;
        // 借出来免得和 &mut self.rng 打架。
        let mut rng = self.rng.clone();
        for fly in self.flies.iter_mut() {
            match fly.state {
                FlyState::Flying => {
                    let step = fly.speed * 60. * delta_sec;
                    let mut next = false;
                    let mut out = false;
                    // x 这一轴。
                    if fly.x < fly.target.0 {
                        fly.x += step;
                        if fly.x >= FIELD_W - FLY_SIZE {
                            (out, next) = (true, true);
                        } else if fly.x >= fly.target.0 {
                            fly.x = fly.target.0;
                            next = true;
                        }
                    } else if fly.x > fly.target.0 {
                        fly.x -= step;
                        if fly.x <= FLY_SIZE {
                            (out, next) = (true, true);
                        } else if fly.x <= fly.target.0 {
                            fly.x = fly.target.0;
                            next = true;
                        }
                    }
                    // y 这一轴，只有 x 没触发换点时才走（原版 `break` 的效果）。
                    if !next {
                        if fly.y < fly.target.1 {
                            fly.y += step;
                            if fly.y >= FIELD_H - FLY_SIZE {
                                (out, next) = (true, true);
                            } else if fly.y >= fly.target.1 {
                                next = true;
                            }
                        } else if fly.y > fly.target.1 {
                            fly.y -= step;
                            if fly.y <= FLY_SIZE {
                                (out, next) = (true, true);
                            } else if fly.y <= fly.target.1 {
                                next = true;
                            }
                        }
                    }
                    if out {
                        fly.edges += 1;
                        if fly.edges >= EDGE_LIMIT {
                            fly.state = FlyState::Done;
                            fly.edges = 0;
                        }
                    }
                    if next {
                        fly.target = (
                            (rng.rand15() % FIELD_W as i32) as f32,
                            (rng.rand15() % FIELD_H as i32) as f32,
                        );
                    }
                }
                FlyState::Falling => {
                    fly.y += fly.fall * 60. * delta_sec;
                    fly.fall += FALL_ACCEL;
                    if fly.y >= floor {
                        // 落地才算钱。
                        fly.state = FlyState::Done;
                        self.earned += fly.value;
                        self.killed += 1;
                    }
                }
                FlyState::Gone | FlyState::Done => {}
            }
        }
        self.rng = rng;
    }

    /// 这一波是不是打完了。
    pub fn finished(&self) -> bool {
        self.flies.iter().all(|f| f.state == FlyState::Done)
    }
}

/// 开局要花多少精：`ftol(当前精 × 0.01)`，至少 1；精不足 1 就是 0
/// （`FUN_00529870` 的 0x529970 一段）。
pub fn cost_from_hp(hp: i32) -> i32 {
    if hp < 1 {
        return 0;
    }
    ((hp as f32 * 0.01) as i32).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cost_is_one_percent_of_current_hp() {
        assert_eq!(cost_from_hp(0), 0);
        assert_eq!(cost_from_hp(-5), 0);
        assert_eq!(cost_from_hp(50), 1, "不足 1 也要收 1");
        assert_eq!(cost_from_hp(100), 1);
        assert_eq!(cost_from_hp(999), 9);
    }

    #[test]
    fn a_wave_is_one_fly_or_three_with_a_gold_one() {
        // 掷一百局，只应出现 1 只或 3 只；三只那一波第一只是金的。
        for seed in 0..100u32 {
            let g = FlyGame::new(seed.wrapping_mul(0x9e37_79b9) | 1);
            assert!(
                g.flies.len() == 1 || g.flies.len() == GOLD_WAVE_FLIES,
                "seed {seed} 出了 {} 只",
                g.flies.len()
            );
            if g.flies.len() == GOLD_WAVE_FLIES {
                assert!(g.flies[0].gold);
                assert!((100..200).contains(&g.flies[0].value));
                assert!(g.flies[1..].iter().all(|f| !f.gold));
            }
            for f in &g.flies {
                if !f.gold {
                    assert!((1..=20).contains(&f.value), "普通苍蝇值 {}", f.value);
                }
                assert!((0.19..=1.31).contains(&f.speed), "速度 {}", f.speed);
            }
        }
    }

    #[test]
    fn a_swatted_fly_only_pays_when_it_lands() {
        let mut g = FlyGame::new(1);
        // 摆一只固定的，好算。
        g.flies = vec![Fly {
            x: 100.,
            y: 100.,
            target: (100., 100.),
            speed: 1.,
            value: 7,
            gold: false,
            state: FlyState::Flying,
            fall: 0.,
            edges: 0,
        }];
        assert!(g.swat(100., 100.));
        assert_eq!(g.earned, 0, "刚拍中还没落地，不给钱");
        for _ in 0..600 {
            g.tick(0.05);
        }
        assert_eq!(g.flies[0].state, FlyState::Done);
        assert_eq!(g.earned, 7);
        assert_eq!(g.killed, 1);
        assert!(g.finished());
    }

    #[test]
    fn a_fly_that_leaves_pays_nothing() {
        let mut g = FlyGame::new(2);
        g.flies = vec![Fly {
            x: 10.,
            y: 10.,
            target: (-500., -500.),
            speed: 1.3,
            value: 20,
            gold: false,
            state: FlyState::Flying,
            fall: 0.,
            edges: 0,
        }];
        for _ in 0..2000 {
            g.tick(0.05);
            if g.finished() {
                break;
            }
        }
        assert!(g.finished());
        assert_eq!(g.earned, 0);
        assert_eq!(g.killed, 0);
    }

    #[test]
    fn a_miss_hits_nothing() {
        let mut g = FlyGame::new(3);
        g.flies = vec![Fly {
            x: 400.,
            y: 300.,
            target: (400., 300.),
            speed: 1.,
            value: 5,
            gold: false,
            state: FlyState::Flying,
            fall: 0.,
            edges: 0,
        }];
        assert!(!g.swat(0., 0.));
        assert_eq!(g.flies[0].state, FlyState::Flying);
    }
}
