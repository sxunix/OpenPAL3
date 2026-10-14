//! 场景物件的动作队列和会动的机关（`FUN_00424c20` 排动作、`FUN_0042ebe0` 开始一条、`FUN_0042f460` 每帧推）。
//!
//! 原版每个物件带一个动作队列（物件 +0x340 起的 deque，每条 0x4c 字节：+0 类型、+4 时长、+8 已过的秒数、
//! +0xc 起点、+0x18 终点、+0x3c / +0x40 两个参数）。物件收到事件（`FUN_00424c20`：0 主角站在它压住的格子上、
//! 2 调查或被连着的物件叫、3 跳下来落地）时按类型往队列里排一串动作，然后 `FUN_0042ebe0` 从队头做起：能当场
//! 做完的做完出队、接着做下一条，要等的就停在那。每帧 `FUN_00426eb0` 先推动画（`FUN_00429d10`），再
//! `FUN_0042f460` 推队头那条（已过的秒数加这一帧，到了就出队、接着 `FUN_0042ebe0`），最后按类型跑物件自己的
//! 状态（+0x50，计时 +0x54）。物件不在场（+0xad 为 0）整个跳过。
//!
//! 排队的前提都是 `FUN_0042c1a0`：队列空、状态 0。乌龟沉下去（状态 3）、吊桥放下（状态 1）以后就不会再动。
//!
//! 这里做了吊桥（0x13）、乌龟重力机关（0x1d）、神树传送点（0x26）、上下传送板（0x28），以及调查键（事件 1）
//! 触发的主角特技机关（0xc）和开关（0x17），古藤林的食人花（0x20），蜀山故道的踏板开关（0x21），神魔之井的电梯（0x27），灵山仙人洞的旋转石梁（7、8），草海的坠落障碍物（0x24），黄泉路的推箱子（0xb）和升降机关门（0x1e），冰风谷的滑道（0x23）。动作里用到主角、模型、音效、脚本、冻结计数的地方都经 [`World`]
//! 交给调用方。
//!
//! `FUN_00424c20` 开头先看可用次数（+0xae）：不是 0xff（无限）又已经是 0 的，什么事件都不理。

use std::collections::VecDeque;

use radiance::math::Vec3;

/// 动作 7 的时间倍率（`gbVisNode::SetTimeScale(node, 2.0)`）。
pub const ANIM_SPEED: f32 = 2.0;
/// 乌龟：主角离它（x、z）超过这么远不理（`FUN_00424c20` case 0x1d）。
pub const TURTLE_REACH: f32 = 36.;
/// 乌龟往下沉多少、沉多久（同上，`y − 7`、2.5 秒）。
pub const TURTLE_SINK: f32 = 7.;
pub const TURTLE_SINK_SECS: f32 = 2.5;
/// 乌龟状态 2 过这么久变 3（`FUN_00426eb0` case 0x1d）。
pub const TURTLE_SETTLE_SECS: f32 = 2.5;
/// 神树传送点：站满这么久才动、升降多高、多久（`FUN_00424c20` case 0x26）。
pub const PORTAL_DWELL_SECS: f32 = 4.;
pub const PORTAL_RISE: f32 = 200.;
pub const PORTAL_SECS: f32 = 3.;
/// 上下传送板挪到另一层要多久（case 0x28，`0x41000000`）。
pub const PEDAL_SECS: f32 = 8.;
/// 神魔之井电梯升降要多久（case 0x27，`0x41000000`）。
pub const ELEVATOR_SECS: f32 = 8.;
/// 旋转石梁每秒转多少度、一次转多少（`FUN_00426eb0` case 7 的 `dt × 30`，`FUN_00424c20` case 7 的 `+ 90`）。
pub const BEAM_DEG_PER_SEC: f32 = 30.;
pub const BEAM_TURN_DEG: f32 = 90.;
/// 推箱子：按正操作的主角（景天、雪见、龙葵、紫萱、长卿）往外再让多少（`FUN_00424c20` case 0xb 的
/// `{-12, 4, 4, 9, 12}`），推一格的路程、推多久封顶（`50 / WalkSpeed`）。
pub const BOX_ROLE_OFFSET: [f32; 5] = [-12., 4., 4., 9., 12.];
pub const BOX_STEP: f32 = 50.;
/// 主角推箱子的动作（`ACT_PUSH`，`+0x504 = 7`）。
pub const ACT_PUSH: usize = 7;
/// 三态箱子推完（第 `推的方向 − 2` 行、现在的状态那一列）换成的状态，`0xff` 是推不动的那一格
/// （`FUN_0042afa0` 栈上 `{2, 0, 0xff, 1, 0xff, 0}`）。
const BOX_NEXT_STATE: [[u8; 3]; 2] = [[2, 0, 0xff], [1, 0xff, 0]];
/// 滑道：主角每秒滑多远（`FUN_0042ebe0` case 0xc 的 `总长 × 0.0016666667`），滑的时候换的动作。
pub const SLIDE_SPEED: f32 = 600.;
pub const ACT_SLIDE: usize = 2;
/// 参数 0 为 1 的移动浮板（海底城）：每秒走多远（`FUN_00426eb0` case 5 的 `段长 × 0.011111111`），到点后
/// 往前看多远找落脚的地方（`144.0` = `ROLE_RADIUS × 12`）。
pub const HOP_SPEED: f32 = 90.;
pub const HOP_LOOKAHEAD: f32 = ROLE_RADIUS * 12.;
/// 升降机关门升 / 降多久（`FUN_00426eb0` case 0x1e 的 `2.0`）。
pub const DOOR_SECS: f32 = 2.;

/// 动作 0xd（落下）的重力加速度（`FUN_0042f460` case 0xd 的 `dt × 490`），落到所在格地面下这么多停。
pub const FALL_GRAVITY: f32 = 490.;
pub const FALL_BELOW_FLOOR: f32 = 4.;
/// 调查够得着的距离（`FUN_00424c20` case 0xc / 0x17 的 `600.0`，源码 `TOUCH_DIST`）。
pub const TOUCH_DIST: f32 = 600.;
/// `ROLE_RADIUS`（12）。特技机关：主角离机关（x、z）不到 3 倍就地施展，否则走到机关另一侧 12 处。
pub const ROLE_RADIUS: f32 = 12.;
/// 开关：主角离开关（x、z）减去半边长不到这么多（`ROLE_RADIUS × 2`）就地拨，否则先走过去。
pub const SWITCH_REACH: f32 = 24.;
/// 动作 0x13（`FUN_0042ebe0`）：走完以后离物件（减半边长）超过这么远就把队列清掉、提示「outdist」。
/// [推断] exe 里读的是 `_DAT_00d8bef0`，没找到初始化它的地方；值取源码 `SceneObj_Common.cpp:278`
/// 的 `DIST_RESTRICT = ROLE_RADIUS * 9`。
pub const DIST_RESTRICT: f32 = ROLE_RADIUS * 9.;
/// 开关参数 2 为 2 时，`p[2]` 起的三个参数是世界转到的角度（状态 11 用 5 秒转过去）。[未做]
pub const ROTATE_SECS: f32 = 5.;
/// 主角动作编号（`ACTION_KIND`，名字见 [`crate::openpal3::combat::motion::ACTION_NAMES`]）：技能、调查。
pub const ACT_SKILL: usize = 8;
pub const ACT_CHECK: usize = 0xb;
/// 开关拨到一半的状态（`FUN_00424c20` 排的动作 3 参数 `0x4d`）：下一帧 `FUN_0042c810` 翻开关、转状态 1。
const SWITCH_PENDING: u8 = 0x4d;

/// 队列里的一条动作。编号是记录 +0 的类型。
#[derive(Debug, Clone)]
pub enum Act {
    /// 1（`FUN_0042c950`）：物件从排队时的位置按余弦缓动挪到 `to`，`dur` 秒；`carry` 把主角一起放到物件的位置
    /// （`FUN_00416490`），`home` 顺带改物件的原位 +0xd4（移植端没有用到原位的地方，不记）。
    Move {
        to: Vec3,
        dur: f32,
        carry: bool,
        home: bool,
    },
    /// 2（`FUN_0042cb70`）：主角走到 `to`（`FUN_00412b60(to, 1)`，按走的速度）；`check` 每步查能不能走，
    /// `fix` 每步按地面重算高度（不重算就保持原来的高度）。
    Walk { to: Vec3, check: bool, fix: bool },
    /// 4（`FUN_0042cf60`）：跑编号 `id` 的 SCE 过程（`FUN_00432170(id, 0)`：编号 < 100 查一张表、否则查当前场景
    /// 那张）；`wait` 等它跑完（`FUN_004324b0`）。[未做] 等：用到的传送点都不等。
    Proc { id: i32, wait: bool },
    /// 3（`FUN_0042cd90`）：物件状态置成 `state`、计时清零（`FUN_0042fb40`）；`wait` 1 等到状态不是 1，
    /// 2 等到状态回 0，0 不等。
    State { state: u8, wait: u8 },
    /// 5（`FUN_0042d130`）：连着的物件（+0x1f4，`.scn` 记录 +0x148）收到事件 2（`FUN_0042bf90`）；
    /// `wait` 等它闲下来（`FUN_0042c1a0`）。
    Link { wait: bool },
    /// 7（`FUN_0042d2f0`）：物件动画从头播、时间倍率 2，等播完（`FUN_0042b1f0`）；`hold` 停在最后一帧。
    Anim { hold: bool },
    /// 8（`FUN_0042d4b0`）：冻结计数（`FUN_0041e720`），`false` 减一、`true` 加一；小于 1 时主角、野怪都停。
    Freeze(bool),
    /// 9（`FUN_0042d670`）：`true` 清掉物件压住的格子、`false` 按现在的位置和层重新登记（`FUN_0042a010`）。
    Tiles(bool),
    /// 10（`FUN_0042d830`）：物件的「可通过」+0xd1。
    Passable(bool),
    /// 0xb（`FUN_0042d9f0`）：主角转向这一点（`FUN_00416550`）。
    Face(Vec3),
    /// 0xe（`FUN_0042df30`）：主角换到第 `layer` 层（`FUN_004125b0`）；`object` 物件也换（+0xf6）。
    Layer { layer: usize, object: bool },
    /// 0x12（`FUN_0042e4d0`）：按物件类型放音效（`FUN_0042c440`）。
    Sound(i32),
    /// 0x14（`FUN_0042e870(秒)`）：等这么久。
    Delay(f32),
    /// 0xd（`FUN_0042dd80` 排）：物件竖直往下掉。开始时（`FUN_0042ebe0`）取物件所在格（按它的层）的地面
    /// 高度减 4 当落点 `to_y`；每帧（`FUN_0042f460`）速度减 `490 × dt`、位置加速度 × dt，掉到落点停住，
    /// 原位 +0xd4 改成现在的位置。
    Fall { to_y: f32, vy: f32 },
    /// 0xc（`FUN_0042dbd0` 排）：主角沿物件的路线滑过去。开始时（`FUN_0042ebe0`）主角站住，路线 =
    /// 主角现在的位置 + 物件的折线（`.scn` +0x84），时长 = 总长 / 600，主角换动作 2、交给机关控制；
    /// 每帧（`FUN_0042f460`）按余弦缓动沿路线取点，主角放过去、朝那一段的方向；到了站住、还回控制。
    PlayerPath { path: Option<PathLine> },
    /// 0x13（`FUN_0042e6c0` 排、`FUN_0042ebe0` 做）：主角离物件（x、z）减去半边长超过 [`DIST_RESTRICT`]
    /// 就清空整个队列、提示「outdist」，否则接着做下一条。
    DistRestrict,
}

#[derive(Debug, Clone)]
struct Entry {
    act: Act,
    /// +4：只有挪物件用。
    dur: f32,
    /// +8。
    elapsed: f32,
    /// +0xc：排队时物件的位置。
    from: Vec3,
}

/// 建队列要的 `.scn` 字段。
#[derive(Debug, Clone)]
pub struct ObjNode {
    pub object_id: i32,
    /// +0xf4。
    pub kind: u8,
    /// 所在层（+0xf6）。
    pub layer: usize,
    /// 参数 0..3（+0xf8..+0x104，`.scn` 记录 +0x4c 起）。
    pub p: [i32; 4],
    /// 连着的物件编号（+0x1f4，`.scn` 记录 +0x148），0xffff 是没有。
    pub link_id: u16,
    /// 物件自己带的音效名（+0x234，`.scn` 记录 +0x188）。非空就用它，不查类型表。
    pub sound: Option<String>,
    /// 物件自己那条 SCE 过程（+0x12c，`.scn` 记录 +0x80）。见 [`ObjectActions::run`] 里动作 5 的说明。
    pub proc: i32,
    /// 可用次数（+0xae，`.scn` 记录 +2）：0xff 无限。
    pub times: u8,
    /// 调查键能不能选中它（`TriggerType` +0xd0 的位 1，`.scn` 记录 +0x24）。
    pub examinable: bool,
    /// 移动浮板（0x5）的路线（`.scn` 记录 +0x84 起的 `PolyLine`：点数 + 16 个点）。别的类型是空的。
    pub path: Vec<Vec3>,
    /// `.scn` 里的位置 y（物件原位 +0xd8；升降平台降到底的高度）。
    pub home_y: f32,
    /// 升降平台（0x16）跟着哪个场景的哪个开关（`depend_scenename` / `depend_id`，`.scn` 记录 +0x14a /
    /// +0x16a）：那个开关开着就直接停在上面。
    pub depend: Option<(String, i32)>,
    /// 开关标志 +0xaf 的初值（`.scn` 记录 +3）。移动浮板从它开始（锁妖塔七层的浮板一开始就是开的）。
    pub switch_init: bool,
}

#[derive(Debug, Clone)]
struct Obj {
    kind: u8,
    sound: Option<String>,
    proc: i32,
    layer: usize,
    p: [i32; 4],
    link: Option<usize>,
    /// +0x50。
    state: u8,
    /// +0x54。
    timer: f32,
    queue: VecDeque<Entry>,
    /// +0xae。
    times: u8,
    examinable: bool,
    /// 登记在 `ObjLockInput` 里（`FUN_0053d1a0(物件, 0)`）：物件闲下来（不在场，或队列空、状态 0，
    /// 即 `IsStatic`）之前主角不听输入。
    locked: bool,
    /// 移动浮板（0x5、参数 0 为 0 的那种）。
    plat: Option<Platform>,
    /// 升降平台（0x16）：原位 y、跟着的开关、载着主角（+0x338）。
    home_y: f32,
    depend: Option<(String, i32)>,
    lift: bool,
    /// 旋转石梁（7、8）要转到的角度（+0x38）。
    beam_to: f32,
    /// `.scn` 的折线（+0x130 点数、+0x13c 起的点），滑道用。
    path: Vec<Vec3>,
    /// 推箱子：三态箱子的状态（+0x51）、推的方向（+0x324，0 往 +x、1 往 −x、2 往 +z、3 往 −z）、
    /// 开推时箱子的位置（+0x38）、每秒挪多少（+0x44）。升降机关门：开动时的位置（+0x38）、
    /// 升降多高（+0x44，模型高 − 2）。
    box_state: u8,
    push_dir: usize,
    push_from: Vec3,
    push_vel: Vec3,
}

/// 移动浮板沿路线来回（`FUN_00426eb0` case 5）。物件状态 +0x50：0 停在起点、1 往终点走、2 往起点走、
/// 3 停在终点、4 主角正走上板、5 主角正走下板。
#[derive(Debug, Clone)]
struct Platform {
    path: PathLine,
    /// 走过的路程（+0x38）。
    dist: f32,
    /// 走上 / 走下板以后转到的状态（+0x324）。
    next: u8,
    /// 载着主角（+0x338）。开关推动时是 false，板子自己跑。
    lift: bool,
    /// 板子自己的开关标志（+0xaf）：连着的开关拨开了才能坐。
    on: bool,
}

/// 浮板每秒走多远（`FUN_00426eb0` case 5 的 `dt × 100`）。
pub const PLATFORM_SPEED: f32 = 100.;
/// 到头以后主角沿路线方向往外走多远（`ROLE_RADIUS × 8` = 96）。
pub const PLATFORM_STEP_OFF: f32 = ROLE_RADIUS * 8.;
/// 板子比主角高过这么多不理（`40.0`）。
pub const PLATFORM_MAX_RISE: f32 = 40.;

/// `PathObj`（`gamebase/roledef.h`、`pathfinder.cpp` 的 `CachePathStats` / `MapPathDistance2Point`；
/// exe `FUN_0041f120`）：折线，按路程取点，方向取那一段的单位向量。
#[derive(Debug, Clone)]
pub struct PathLine {
    points: Vec<Vec3>,
    lengths: Vec<f32>,
    normals: Vec<Vec3>,
    total: f32,
}

impl PathLine {
    pub fn new(points: Vec<Vec3>) -> Self {
        let n = points.len();
        let mut lengths = vec![0.; n];
        let mut normals = vec![Vec3::new(0., 0., 0.); n];
        let mut total = 0.;
        for i in 1..n {
            let d = Vec3::sub(&points[i], &points[i - 1]);
            let len = d.norm();
            lengths[i] = len;
            normals[i] = if len > 0. {
                Vec3::scalar_mul(1. / len, &d)
            } else {
                d
            };
            total += len;
        }
        if n > 1 {
            normals[0] = normals[1];
            lengths[0] = lengths[1];
        }
        Self {
            points,
            lengths,
            normals,
            total,
        }
    }

    pub fn total(&self) -> f32 {
        self.total
    }

    /// 路程 `d` 上的点和那一段的方向。小于 0 取起点，超长取终点。
    pub fn at(&self, d: f32) -> (Vec3, Vec3) {
        let n = self.points.len();
        if n == 0 {
            return (Vec3::new(0., 0., 0.), Vec3::new(0., 0., 1.));
        }
        if d < 0. {
            return (self.points[0], self.normals[n.min(2) - 1]);
        }
        let mut rest = d;
        for i in 1..n {
            let seg = self.lengths[i];
            if seg < rest {
                rest -= seg;
            } else {
                let r = if seg > 0. { rest / seg } else { 0. };
                let (a, b) = (self.points[i - 1], self.points[i]);
                let p = Vec3::new(
                    a.x + (b.x - a.x) * r,
                    a.y + (b.y - a.y) * r,
                    a.z + (b.z - a.z) * r,
                );
                return (p, self.normals[i]);
            }
        }
        (self.points[n - 1], self.normals[n - 1])
    }

    pub fn last(&self) -> Option<Vec3> {
        self.points.last().copied()
    }
}

/// 动作要碰的外部东西。`i` 都是物件在 `.scn` 里的序号。
pub trait World {
    /// +0xad：物件在不在场。
    fn active(&self, i: usize) -> bool;
    fn object_pos(&self, i: usize) -> Vec3;
    fn set_object_pos(&mut self, i: usize, p: &Vec3);
    fn player_pos(&self) -> Vec3;
    /// `FUN_00416490`：主角放到这一点。
    fn carry_player(&mut self, p: &Vec3);
    /// `FUN_00412230(0, 0)`：主角按脚下（连物件）重算高度。
    fn fix_player_height(&mut self);
    fn play_anim(&mut self, i: usize, speed: f32);
    fn anim_done(&self, i: usize) -> bool;
    /// `FUN_0041e720`。
    fn freeze(&mut self, release: bool);
    fn set_passable(&mut self, i: usize, passable: bool);
    fn face_player(&mut self, p: &Vec3);
    /// `FUN_004181e0` + `FUN_00418130(1)` + `FUN_00412b60(to, 1)`。
    fn walk_begin(&mut self, to: &Vec3);
    /// `FUN_0042f460` case 2 推一帧，走完（到了、被挡住、或者一开始就离目标不到 12）返回 `true`。
    fn walk_step(&mut self, dt: f32, check: bool, fix: bool) -> bool;
    fn sound(&mut self, name: &str);
    /// `FUN_00432170(id, 0)`：跑 SCE 过程。
    fn call_proc(&mut self, id: i32);
    /// `FUN_0042a010`：`clear` 清掉物件压住的格子，否则按物件现在的位置在第 `layer` 层重新登记。
    fn object_tiles(&mut self, i: usize, clear: bool, layer: usize);
    /// `FUN_004125b0`：主角换层。
    fn set_player_layer(&mut self, layer: usize);
    /// `FUN_00429db0` 的包围盒 y 边长（+0xa4）。
    fn object_model_height(&self, i: usize) -> f32;
    /// 第 `layer` 层 (`x`, `z`) 格：中心点（y 是格子自己的高度，越界是 0）和连物件的地面高度（`FUN_00421230`）。
    fn tile_point(&self, layer: usize, x: i32, z: i32) -> (Vec3, f32);
    /// 物件的状态（+0x50）变了，记进存档（原版靠物件的「被改过」标记 +0x240，存档里带着状态）。
    fn save_state(&mut self, i: usize, state: u8);
    /// 有脚本过程在跑没有（`FUN_004324b0`：动作 4 参数非 0 时要等它跑完）。
    fn proc_running(&self) -> bool;
    /// `FUN_0040fc70`：队里几个人。
    fn party_size(&self) -> usize;
    /// `FUN_00430570(key)` + `FUN_00450bc0`：弹一句提示。
    fn hint(&mut self, key: &str);

    // 下面是调查键那两种物件（0xc、0x17）才用到的；测试替身不用的都给了不起作用的缺省。

    /// `DAT_00c10cc8`（`PlayerMgr::m_underctrl`）：正操作的是哪个主角。
    fn controlled_role(&self) -> i32 {
        0
    }
    /// 主角在第几层（+0x274）。
    fn player_layer(&self) -> usize {
        0
    }
    /// `FUN_0042ac80`（`SceneObj::CanRoleGoto`）：在物件所在的第 `layer` 层上，两点所在格之间按 Bresenham
    /// 一格格都能走——物件自己压住的格子不算挡（`Scene::Obj_CanDirectGo`）。
    fn can_role_goto(&self, _i: usize, _layer: usize, _from: &Vec3, _to: &Vec3) -> bool {
        true
    }
    /// 物件模型包围盒的 x、z 边长（`GetRawOBB`）；没有模型是 `None`。
    fn object_extent(&self, _i: usize) -> Option<(f32, f32)> {
        None
    }
    /// 物件是不是带动画的模型（模型类型 3）。不是的话播动画那一步当场算完。
    fn object_animated(&self, _i: usize) -> bool {
        true
    }
    /// `FUN_004181e0`：主角站住。
    fn stand(&mut self) {}
    /// `FUN_00414e30(act, 1)`：主角把动作 `act` 播一遍。
    fn show_act(&mut self, _act: usize) {}
    /// 主角的 `PalScriptWaitObj::IsWaiting`（动作还没播完）。
    fn player_acting(&self) -> bool {
        false
    }
    /// `FUN_0042c810` 里那句 `+0xaf = !+0xaf`：翻开关，记进存档（126 `GetSwitch` 读的就是它）。返回新值。
    fn toggle_switch(&mut self, _i: usize) -> bool {
        false
    }
    /// 开关（+0xaf）现在是不是开着。
    fn switch_on(&self, _i: usize) -> bool {
        false
    }
    /// 物件不在场了（+0xad 置 0），记进存档。
    fn deactivate(&mut self, _i: usize) {}
    /// `FUN_00414e50(to, flag)`（`Role::RotateFly`）：主角站住，0.5 秒里按余弦缓动从现在的位置飞到 `to`；
    /// `flag` 1 是飞进去（缩到 0.7），2 是飞出来（从 0.7 放回 1）。飞的时候算「动作没播完」
    /// （[`World::player_acting`]）。
    fn fly_player(&mut self, _to: &Vec3, _flag: u8) {}
    /// 主角在不在场（`+0x278`）：被食人花吞进去时是 0。
    fn set_player_shown(&mut self, _shown: bool) {}
    /// `FUN_004163b0(x, z)`：主角放到第 `layer` 层 (`x`, `z`) 格，再把渲染位置挪到 `at`
    /// （主角 `+0xa0`，食人花取配对那朵花的位置）。
    fn place_player(&mut self, _layer: usize, _x: i32, _z: i32, _at: &Vec3) {}
    /// 物件模型停回第一帧、不再播（`SetTimeScale(0)` + `Reset(0)`）。
    fn reset_anim(&mut self, _i: usize) {}
    /// 可用次数（+0xae）变了，记进存档。[推断] 原版存档里那条物件记录就是 `ObjectInfo` 本身
    /// （源码 `SceneObj::OnLoad(ObjectInfo &info, …)`，`gamebase/sceneobj.cpp:4002`），次数在里面。
    fn save_times(&mut self, _i: usize, _times: u8) {}
    /// 神树花总开关翻了：记下花开没开（`DAT_00d8bc38`），把场上的花都换过去。
    fn switch_flowers(&mut self, _open: bool) {}
    /// 当前城里 `scene` 那个子场景的 `id` 号物件，开关标志开着没有（`Archive::FindObj`，存档里那条）。
    fn depend_switch_on(&self, _scene: &str, _id: i32) -> bool {
        false
    }
    /// 主角站在物件 `i` 压住的格子上（`FUN_0042b450` / `IfOnMe`：同一层、脚下那格登记的是它）。
    fn player_on(&self, _i: usize) -> bool {
        false
    }
    /// 物件 `i` 的位置记进存档（升降平台停住时）。
    fn save_pos(&mut self, _i: usize) {}
    /// 物件 `i` 自己的开关标志 +0xaf 改成 `on`（开关推动移动浮板时写浮板的）。
    fn set_switch(&mut self, _i: usize, _on: bool) {}
    /// `FUN_00421150(i, 层, x, z)`：物件 `i` 眼里 `p` 那一格能不能走（它自己压住的格子只看格子本身）。
    fn object_tile_walkable(&self, _i: usize, _p: &Vec3) -> bool {
        true
    }
    /// 物件的触发矩形（`.scn` +0x38，格子坐标 x0 z0 x1 z1，物件 +0xe4..+0xf0）。
    fn object_rect(&self, _i: usize) -> [i32; 4] {
        [0; 4]
    }
    /// 第 `layer` 层在 `p` 这一点（x、z 落的那格）的地面高度（`FUN_00421440(layer, p, 0, 0)`）。
    /// 物件绕 Y 的角度（`m_info.rotate`，+0xe0），度。
    fn object_yaw(&self, _i: usize) -> f32 {
        0.
    }
    fn set_object_yaw(&mut self, _i: usize, _deg: f32) {}
    /// 角度进存档（原版整份 `ObjectInfo` 进存档，石梁转过的角度跟着存）。
    fn save_yaw(&mut self, _i: usize) {}
    /// 导航格的原始地面高度（`FUN_00420f30` 那一层格子记录的 +0），不算物件；格子出界是 0。
    fn nav_floor(&self, _layer: usize, _p: &Vec3) -> f32 {
        0.
    }
    /// `FUN_00420320(物件触发矩形, 0xff)`：物件所在层上它的触发矩形清空。
    fn clear_trigger_rect(&mut self, _i: usize) {}
    /// 推箱子 `FUN_0042a950`：箱子 `i` 挪到 `at` 行不行。
    fn box_can_go(&self, _i: usize, _at: &Vec3) -> bool {
        true
    }
    /// 物件 `i` 在第 `layer` 层压住的格子的外接矩形（`m_maprect`）。
    fn object_owned_rect(&self, _i: usize, _layer: usize) -> Option<[i32; 4]> {
        None
    }
    /// 主角在第 `layer` 层的哪一格。
    fn player_tile(&self, _layer: usize) -> (i32, i32) {
        (0, 0)
    }
    /// NPC 走路速度 `WalkSpeed`（`DAT_0058b38c`，`datascript\scene.txt` 出厂 80）。
    fn npc_walk_speed(&self) -> f32 {
        80.
    }
    /// 主角换成动作 `act` 一直播（`m_curact = act; OnChangeAct()`）。
    fn set_act(&mut self, _act: usize) {}
    /// 主角原样放到 `p`（带高度，不按地面重算；站在浮着的物件上用）。
    fn put_player(&mut self, p: &Vec3) {
        self.carry_player(p);
    }
    /// 第 `layer` 层 `p` 那一格能不能走（`FUN_00420fb0(层, 点, 0xff)`）。
    fn walkable_at(&self, _layer: usize, _p: &Vec3) -> bool {
        true
    }
    fn floor_at(&self, _layer: usize, _p: &Vec3) -> f32 {
        0.
    }
    /// 只挪模型（渲染位置 +0x28），不进存档：剑冢的锤上下动时原版改的是 `m_renderpos`，存档里的
    /// `m_info.pos` 不动。
    fn set_render_pos(&mut self, i: usize, p: &Vec3) {
        self.set_object_pos(i, p);
    }
    /// 左右摆的剑：模型绕自己的 Z 轴转到 `deg`（+0x38，`FUN_00429ee0`）。
    fn set_object_roll(&mut self, _i: usize, _deg: f32) {}
    /// 离上次被机关打到超过 1.2 秒（`_DAT_0058de64`，源码 `SceneMgr::m_fHurtTime > HURT_DELAY`）。
    fn hurt_ready(&self) -> bool {
        false
    }
    /// 物件 `i` 现在的碰撞盒碰到主角没有（`FUN_00429ee0` + `FUN_00418170`）。
    fn player_hit_by(&self, _i: usize) -> bool {
        false
    }
    /// `FUN_004101f0(frac, 1, 0)`（`PlayerMgr::Obj_Hurt`）：全队每人扣最大精的 `frac`，最少剩 1；
    /// 受伤计时清零。
    fn obj_hurt(&mut self, _frac: f32) {}
}

/// 场景里所有物件的队列和状态，按 `.scn` 序号排。
#[derive(Debug, Clone, Default)]
pub struct ObjectActions {
    ids: Vec<i32>,
    objs: Vec<Obj>,
}

impl ObjectActions {
    pub fn new(nodes: &[ObjNode]) -> Self {
        let ids: Vec<i32> = nodes.iter().map(|n| n.object_id).collect();
        let objs = nodes
            .iter()
            .map(|n| Obj {
                kind: n.kind,
                sound: n.sound.clone(),
                proc: n.proc,
                layer: n.layer,
                p: n.p,
                // `SceneObj::LinkObject`（`gamebase/sceneobj.cpp:4291`，exe 里
                // `FUN_0042bf90`）：`linkobj != 0xFFFF` 时拿它去 `Scene::GetObjByID`
                // （`FUN_00421560`）找。那边比的是**整个 16 位**——
                // `movzx bx, byte ptr [edx]` 取物件的 `BYTE id`，再 `cmp bx, di`
                // 跟传进来的 WORD 比，所以高字节非零就一个都配不上。
                // 实机 533 个 `.scn` 里 92 条非 0xFFFF 的连接，高字节全是 0，
                // 这条路上永远走得通。
                link: (n.link_id != 0xffff)
                    .then(|| ids.iter().position(|id| *id == n.link_id as i32))
                    .flatten(),
                state: 0,
                timer: 0.,
                queue: VecDeque::new(),
                times: n.times,
                examinable: n.examinable,
                locked: false,
                // `SceneObj::Create` case 5（`sceneobj.cpp:197`）：有路线就建 `PathObj`，状态 0。
                home_y: n.home_y,
                depend: n.depend.clone(),
                lift: false,
                beam_to: 0.,
                path: n.path.clone(),
                box_state: 0,
                push_dir: 0,
                push_from: Vec3::new(0., 0., 0.),
                push_vel: Vec3::new(0., 0., 0.),
                plat: (n.kind == 5 && n.p[0] == 0 && n.path.len() >= 2).then(|| Platform {
                    path: PathLine::new(n.path.clone()),
                    dist: 0.,
                    next: 0,
                    lift: false,
                    on: n.switch_init,
                }),
            })
            .collect();
        Self { ids, objs }
    }

    pub fn index_of(&self, object_id: i32) -> Option<usize> {
        self.ids.iter().position(|id| *id == object_id)
    }

    pub fn state(&self, i: usize) -> Option<u8> {
        self.objs.get(i).map(|o| o.state)
    }

    /// 读档 / 换场景回来时把状态摆回去（原版把物件状态存进存档，见 `FUN_00538fa0`）。
    pub fn set_state(&mut self, i: usize, state: u8) {
        if let Some(o) = self.objs.get_mut(i) {
            o.state = state;
            o.timer = 0.;
        }
    }

    /// 可用次数（+0xae）。
    pub fn times(&self, i: usize) -> Option<u8> {
        self.objs.get(i).map(|o| o.times)
    }

    /// 读档 / 换场景回来时把次数摆回去。
    pub fn set_times(&mut self, i: usize, times: u8) {
        if let Some(o) = self.objs.get_mut(i) {
            o.times = times;
        }
    }

    /// 物件在第几层（+0xf6）。
    pub fn layer(&self, i: usize) -> Option<usize> {
        self.objs.get(i).map(|o| o.layer)
    }

    /// 物件参数 0..3（+0xf8..+0x104）。
    pub fn params(&self, i: usize) -> Option<[i32; 4]> {
        self.objs.get(i).map(|o| o.p)
    }

    /// 物件的类型（+0xf4）。
    pub fn kind(&self, i: usize) -> Option<u8> {
        self.objs.get(i).map(|o| o.kind)
    }

    /// 调查键选物件那一轮（`FUN_00420cd0`）要先过的几条：调查得了（`TriggerType` 位 1）、次数没用完。
    /// 在场、同层由调用方看。
    pub fn examinable(&self, i: usize) -> bool {
        self.objs
            .get(i)
            .is_some_and(|o| o.examinable && o.times != 0)
    }

    /// 物件那半个 `ObjLockInput`：还有登记着、没闲下来的物件。
    pub fn input_locked(&self) -> bool {
        self.objs.iter().any(|o| o.locked)
    }

    /// [调试] 不闲着的物件：（序号, 类型, 状态, 锁着, 队列长）。
    pub fn report(&self) -> Vec<(usize, u8, u8, bool, usize)> {
        self.objs
            .iter()
            .enumerate()
            .filter(|(_, o)| o.locked || o.state != 0 || !o.queue.is_empty())
            .map(|(i, o)| (i, o.kind, o.state, o.locked, o.queue.len()))
            .collect()
    }

    /// 换场景时清掉（`ObjLockInput::OnLoadScene`）。
    pub fn clear_locks(&mut self) {
        for o in &mut self.objs {
            o.locked = false;
        }
    }

    /// `FUN_0042c1a0`：队列空、状态 0。
    pub fn idle(&self, i: usize) -> bool {
        self.objs
            .get(i)
            .is_some_and(|o| o.queue.is_empty() && o.state == 0)
    }

    /// `SceneObj::IsStatic`（`sceneobj.cpp:4391`）：移动浮板停在两头（状态 0 / 3）都算静止，别的看
    /// [`Self::idle`]。`ObjLockInput` 按它放人。
    pub fn is_static(&self, i: usize) -> bool {
        match self.objs.get(i) {
            Some(o) if o.kind == 5 => o.queue.is_empty() && (o.state == 0 || o.state == 3),
            Some(o) if o.kind == 0x16 => o.queue.is_empty() && (o.state == 0 || o.state == 2),
            // 坠落的障碍物只看队列空（`sceneobj.cpp:4404`），掉下来以后停在状态 2。
            Some(o) if o.kind == 0x24 => o.queue.is_empty(),
            _ => self.idle(i),
        }
    }

    /// 移动浮板的开关标志（读档时照存档套）。
    pub fn set_platform_switch(&mut self, i: usize, on: bool) {
        if let Some(p) = self.objs.get_mut(i).and_then(|o| o.plat.as_mut()) {
            p.on = on;
        }
    }

    /// 移动浮板：开关开着、停在两头（agent 用）。
    pub fn platform_ready(&self, i: usize) -> bool {
        self.objs
            .get(i)
            .and_then(|o| o.plat.as_ref().map(|p| (o, p)))
            .is_some_and(|(o, p)| p.on && o.queue.is_empty() && (o.state == 0 || o.state == 3))
    }

    /// 移动浮板路线上的点（agent 用）。
    pub fn platform_path(&self, i: usize) -> Option<Vec<Vec3>> {
        self.objs
            .get(i)?
            .plat
            .as_ref()
            .map(|p| p.path.points.clone())
    }

    /// 移动浮板这一程的终点（停在起点就是路线终点，反之起点）和到那一头时的方向（agent 用）。
    pub fn platform_far_end(&self, i: usize) -> Option<(Vec3, Vec3)> {
        let o = self.objs.get(i)?;
        let p = o.plat.as_ref()?;
        if o.state == 3 {
            let (a, n) = p.path.at(0.);
            Some((a, Vec3::scalar_mul(-1., &n)))
        } else {
            Some(p.path.at(p.path.total()))
        }
    }

    /// 队列里还有没有东西（主角被动作牵着走时冒险输入要让开）。
    pub fn busy(&self) -> bool {
        self.objs.iter().any(|o| !o.queue.is_empty())
    }

    /// `SceneObj::InterActRole` 的末尾（`gamebase/sceneobj.cpp:1559`）：
    /// 大 `switch(m_info.type)` 里凡是 `break` 出来的类型，最后都会无条件
    /// 跑一句 `PalScriptMgr::RunScript(m_info.scriptid)`——**和有没有连接无关**，
    /// 也不进 `Common_*` 队列，是当场跑。`return` 出去的那些（0、15 带脚本、
    /// 16、21、30、以及各种条件不满足的提前 return）跳过这一句。
    ///
    /// 这里模型化的四种里：吊桥 0x13、乌龟 0x1d、上下传送板 0x28 是
    /// `break`（要跑），神树传送点 0x26 是 `return`（不跑，它那两个过程
    /// 走的是参数 0 / 参数 1）。
    ///
    /// 实机 533 个 `.scn` 里这四种带非零过程的只有两条：M24/2 的 10 号和
    /// M24/3 的 6 号乌龟（过程 506 / 505，都没有连接）。506 的内容是对五个
    /// 巨锤（物件 5..9）各来一次「与物件交互」——一个连接位放不下五个目标。
    fn own_proc<W: World>(&self, i: usize, w: &mut W) {
        let proc = self.objs[i].proc;
        if proc != 0 {
            w.call_proc(proc);
        }
    }

    /// `FUN_00424c20`：物件 `i` 收到事件 `ev`；`same` 是上一帧主角也站在它上面，`dt` 是这一帧的秒数。
    pub fn event<W: World>(&mut self, i: usize, ev: u8, same: bool, dt: f32, w: &mut W) {
        let Some(o) = self.objs.get(i) else {
            return;
        };
        if o.times == 0 {
            return;
        }
        let (kind, p, layer) = (o.kind, o.p, o.layer);
        let p0 = p[0];
        match kind {
            // 吊桥：只认事件 2。主角转过去看、放下（动画停在最后一帧）、变成可通过。
            0x13 => {
                if ev != 2 || !self.idle(i) {
                    return;
                }
                let p = w.object_pos(i);
                for a in [
                    Act::Freeze(false),
                    Act::Face(p),
                    Act::Sound(0),
                    Act::Anim { hold: true },
                    Act::Passable(true),
                    Act::Freeze(true),
                    Act::State { state: 1, wait: 0 },
                ] {
                    self.push(i, a, w);
                }
                self.run(i, w);
                self.own_proc(i, w);
            }
            // 乌龟：主角站上来（事件 0）。队里人不够参数 0 就提示「lessweight」；离乌龟（x、z）超过 36 不理；
            // 否则主角走到乌龟中间、乌龟播动画沉 7、叫连着的物件（吊桥）。
            0x1d => {
                if ev != 0 || !self.idle(i) {
                    return;
                }
                if (w.party_size() as i32) < p0 {
                    // 原版站着的每一帧都弹一次；移植端提示框没做，只在刚站上来那帧记一次。
                    if !same {
                        w.hint("lessweight");
                    }
                    return;
                }
                let p = w.object_pos(i);
                let pl = w.player_pos();
                let (dx, dz) = (pl.x - p.x, pl.z - p.z);
                if (dx * dx + dz * dz).sqrt() > TURTLE_REACH {
                    return;
                }
                let down = Vec3::new(p.x, p.y - TURTLE_SINK, p.z);
                for a in [
                    Act::Freeze(false),
                    Act::Walk {
                        to: p,
                        check: true,
                        fix: true,
                    },
                    Act::Sound(1),
                    Act::Anim { hold: true },
                    Act::State { state: 1, wait: 0 },
                    Act::Sound(0),
                    Act::Move {
                        to: down,
                        dur: TURTLE_SINK_SECS,
                        carry: false,
                        home: true,
                    },
                    Act::Link { wait: false },
                    Act::Freeze(true),
                ] {
                    self.push(i, a, w);
                }
                self.run(i, w);
                self.own_proc(i, w);
            }
            // 神树传送点：站在上面（事件 0、参数 0 非 0、上一帧也站着）满 4 秒，连主角升 200、跑参数 0 那个过程；
            // 跳下来落在上面（事件 3）且参数 1 非 0，降 200、跑参数 1。离开时计时清零（`FUN_0042af80`）。
            0x26 => {
                if !self.idle(i) {
                    return;
                }
                let o = &mut self.objs[i];
                let (dy, id) = match ev {
                    0 => {
                        if p0 == 0 || !same {
                            return;
                        }
                        o.timer += dt;
                        if o.timer < PORTAL_DWELL_SECS {
                            return;
                        }
                        (PORTAL_RISE, p0)
                    }
                    3 => {
                        if p[1] == 0 {
                            return;
                        }
                        (-PORTAL_RISE, p[1])
                    }
                    _ => return,
                };
                o.timer = 0.;
                let pos = w.object_pos(i);
                let to = Vec3::new(pos.x, pos.y + dy, pos.z);
                for a in [
                    Act::Freeze(false),
                    Act::Walk {
                        to: pos,
                        check: false,
                        fix: true,
                    },
                    Act::Move {
                        to,
                        dur: PORTAL_SECS,
                        carry: true,
                        home: false,
                    },
                    Act::Proc { id, wait: false },
                    Act::Freeze(true),
                ] {
                    self.push(i, a, w);
                }
                self.run(i, w);
            }
            // 上下传送板：站上来（事件 0）。另一层（(层 + 1) 的奇偶）按层取目标格——0 层用参数 (0, 1)、1 层用 (2, 3)；
            // 主角走到板中间，板连主角挪 8 秒到「目标格的地面高度 + 板的包围盒高」，清格子、主角和板都换层、在新层
            // 重新登记，主角走到目标格中心。
            0x28 => {
                if ev != 0 || !self.idle(i) {
                    return;
                }
                let to_layer = (layer + 1) % 2;
                let (tx, tz) = if to_layer == 0 {
                    (p[0], p[1])
                } else {
                    (p[2], p[3])
                };
                let (center, floor) = w.tile_point(to_layer, tx, tz);
                let pos = w.object_pos(i);
                let up = Vec3::new(pos.x, floor + w.object_model_height(i), pos.z);
                for a in [
                    Act::Freeze(false),
                    Act::Walk {
                        to: pos,
                        check: false,
                        fix: false,
                    },
                    Act::Move {
                        to: up,
                        dur: PEDAL_SECS,
                        carry: true,
                        home: false,
                    },
                    Act::Tiles(true),
                    Act::Layer {
                        layer: to_layer,
                        object: true,
                    },
                    Act::Tiles(false),
                    Act::Walk {
                        to: center,
                        check: false,
                        fix: false,
                    },
                    Act::Freeze(true),
                ] {
                    self.push(i, a, w);
                }
                self.run(i, w);
                self.own_proc(i, w);
            }
            // 神魔之井电梯（`FUN_00424c20` case 0x27）：不看事件、闲着就动。电梯先挪到触发矩形中间那格的中心
            // （高度取那格的地面），主角走过去，放音效，电梯连主角 8 秒挪到参数 0 那一层在这一点的地面高度，
            // 主角换到那一层（电梯自己不换），再往 +z 走 80；登记 `ObjLockInput`，末尾跑自己那条过程。
            0x27 => {
                if !self.idle(i) {
                    return;
                }
                let r = w.object_rect(i);
                let (cx, cz) = ((r[0] + r[2]) / 2, (r[1] + r[3]) / 2);
                let (at, _) = w.tile_point(layer, cx, cz);
                w.set_object_pos(i, &at);
                let to_layer = p0.max(0) as usize;
                let to = Vec3::new(at.x, w.floor_at(to_layer, &at), at.z);
                let next = Vec3::new(to.x, to.y, to.z + 80.);
                for a in [
                    Act::Walk {
                        to: at,
                        check: false,
                        fix: true,
                    },
                    Act::Sound(0),
                    Act::Move {
                        to,
                        dur: ELEVATOR_SECS,
                        carry: true,
                        home: false,
                    },
                    Act::Layer {
                        layer: to_layer,
                        object: false,
                    },
                    Act::Walk {
                        to: next,
                        check: false,
                        fix: true,
                    },
                ] {
                    self.push(i, a, w);
                }
                self.run(i, w);
                self.objs[i].locked = true;
                self.own_proc(i, w);
            }
            0x5 if self.objs[i].p[0] == 1 => self.hop_touch(i, ev, w),
            0x5 => self.platform_touch(i, w),
            0xb => self.push_box_touch(i, ev, w),
            // 滑道（`FUN_00424c20` case 0x23）：主角踩上来（事件 0）、闲着才动。排「主角沿路线滑」、跑自己
            // 那条过程并等它跑完，登记 `ObjLockInput`。
            0x23 => {
                if ev != 0 || !self.idle(i) {
                    return;
                }
                let proc = self.objs[i].proc;
                for a in [
                    Act::PlayerPath { path: None },
                    Act::Proc {
                        id: proc,
                        wait: true,
                    },
                ] {
                    self.push(i, a, w);
                }
                self.objs[i].locked = true;
                self.run(i, w);
            }
            // 坠落的障碍物（`FUN_00424c20` case 0x24）：主角走进触发区（事件 0）或被别的机关叫（事件 2），
            // 闲着才动。主角站住；排「落下」、状态 1（等每帧那边清掉触发区、转 2）、按落下后的位置登记格子、
            // 不可通过、跑自己那条过程（不等）；放音效，开始跑队列，登记 `ObjLockInput`。
            0x24 => {
                if (ev != 0 && ev != 2) || !self.idle(i) {
                    return;
                }
                w.stand();
                let proc = self.objs[i].proc;
                for a in [
                    Act::Fall { to_y: 0., vy: 0. },
                    Act::State { state: 1, wait: 1 },
                    Act::Tiles(false),
                    Act::Passable(false),
                    Act::Proc {
                        id: proc,
                        wait: false,
                    },
                ] {
                    self.push(i, a, w);
                }
                self.sound(i, 0, w);
                self.run(i, w);
                self.objs[i].locked = true;
            }
            // 旋转石梁 / 旋转墙（`FUN_00424c20` case 7、8）：只认被别的机关叫（事件 2），停着（状态 0）才动。
            // 目标角度 = 现在的角度 + 90，主角站住，清掉压住的格子，放音效，扣一次次数，登记 `ObjLockInput`。
            // [未做] 镜头空闲时锁到石梁上（`FUN_0041c1b0`）。
            7 | 8 => {
                if ev != 2 || self.objs[i].state != 0 {
                    return;
                }
                self.objs[i].state = 1;
                self.objs[i].beam_to = w.object_yaw(i) + BEAM_TURN_DEG;
                w.stand();
                w.object_tiles(i, true, layer);
                self.sound(i, 0, w);
                self.use_once(i, w);
                self.objs[i].locked = true;
            }
            // 升降平台（`FUN_00424c20` case 0x16）：只认被别的机关叫（事件 2）。停在下面就往参数 0 那么高
            // 升，停在上面就往原位降，别的状态不理；放音效 Wg005，主角站在它上面就载着，扣一次次数，
            // 登记 `ObjLockInput`。不跑自己那条过程（停住时才跑）。
            0x16 => {
                if ev != 2 {
                    return;
                }
                let o = &mut self.objs[i];
                o.state = match o.state {
                    0 => 1,
                    2 => 3,
                    _ => return,
                };
                self.sound(i, 0, w);
                self.objs[i].lift = w.player_on(i);
                self.use_once(i, w);
                self.objs[i].locked = true;
            }
            0xc => self.skill_gate(i, ev, w),
            0x17 => self.switch(i, ev, w),
            // 剑冢的剑或锤（`FUN_00424c20` case 0x1c）：只认别的机关叫（事件 2）。参数 0 为 1 的锤翻开关，
            // 「可通过」跟着开关走——开着上下动、能从底下钻过去，关着落在地上挡路。
            0x1c => {
                if ev == 2 && self.objs[i].p[0] == 1 {
                    let on = w.toggle_switch(i);
                    w.set_passable(i, on);
                }
            }
            // 踏板开关（`FUN_00424c20` case 0x21）：只认刚踩上来（事件 0、上一帧不在上面）、闲着。主角走到
            // 踏板中心，踏板 2 秒沉下 5（最后一次用时连原位一起改），跑自己那条过程（不等），叫连着的物件，
            // 扣一次次数；登记 `ObjLockInput`。
            0x21 => {
                if ev != 0 || same || !self.idle(i) {
                    return;
                }
                let p = w.object_pos(i);
                let down = Vec3::new(p.x, p.y - 5., p.z);
                let last = self.objs[i].times == 1;
                let proc = self.objs[i].proc;
                for a in [
                    Act::Walk {
                        to: p,
                        check: true,
                        fix: true,
                    },
                    Act::Move {
                        to: down,
                        dur: 2.,
                        carry: false,
                        home: last,
                    },
                    Act::Proc {
                        id: proc,
                        wait: false,
                    },
                    Act::Link { wait: false },
                ] {
                    self.push(i, a, w);
                }
                self.use_once(i, w);
                self.run(i, w);
                self.objs[i].locked = true;
            }
            // 食人花（`FUN_00424c20` case 0x20）：只认踩上来（事件 0）、状态 0。转状态 1、主角飞进花心、
            // 放音效；`break` 出来，末尾照样跑自己那条过程。之后见 [`Self::step_flower`]。
            0x20 => {
                if ev != 0 || self.objs[i].state != 0 {
                    return;
                }
                self.objs[i].state = 1;
                // [移植] 原版飞行时主角是特殊动作模式、被吞时不在场，都不听输入；这里借
                // `ObjLockInput` 那一位锁住，回到状态 0 时放开。
                self.objs[i].locked = true;
                let p = w.object_pos(i);
                w.fly_player(&p, 1);
                self.sound(i, 0, w);
                self.own_proc(i, w);
            }
            _ => {}
        }
    }

    /// `FUN_00426eb0` case 0x16：升（1）每秒 30 到参数 0，降（3）每秒 30 到原位。到了：停住（2 / 0）、
    /// 可通过，参数 1 非 0 就换层（`FUN_0042b510`：上面是 1 层、下面是 0 层，载着的主角一起换），
    /// 排「等 2 秒」；每帧 `FUN_0042b350` 把参数 2 那个物件的高度带上、层不同就挪过来，开着
    /// `depend` 那个开关时直接摆到上面。停住那一帧：不再载人，叫连着的物件，跑自己那条过程。
    /// [未做] 镜头锁主角（`FUN_0042ea30`）。
    /// `FUN_00426eb0` case 7、8：状态 1 时每秒转 30 度，过了目标就停在目标、回状态 0、按新角度登记格子，
    /// 排 2 秒延时（`FUN_0042e870`），接着跑队列（`FUN_0042ebe0`）。连着的不是另一根石梁时原版还排一条
    /// 镜头回到主角（`FUN_0042ea30`）——[未做]，移植端镜头没锁过去。
    fn step_beam<W: World>(&mut self, i: usize, dt: f32, w: &mut W) {
        if self.objs[i].state != 1 {
            return;
        }
        let to = self.objs[i].beam_to;
        let yaw = w.object_yaw(i) + dt * BEAM_DEG_PER_SEC;
        if yaw <= to {
            w.set_object_yaw(i, yaw);
            return;
        }
        w.set_object_yaw(i, to);
        self.objs[i].state = 0;
        let layer = self.objs[i].layer;
        w.object_tiles(i, false, layer);
        w.save_yaw(i);
        self.push(i, Act::Delay(2.), w);
        self.run(i, w);
    }

    /// `FUN_00424c20` case 0xb（`SceneObj::InterActRole` 推箱子）：只认调查（事件 1）、停着才动。箱子四边各
    /// 一个推点：x、z 方向离箱子位置「模型盒子那一边的长 + 按主角让的那点」，取离主角（x、z）最近的；
    /// 不到 [`TOUCH_DIST`] 且走得过去才推，否则提示「outdist」。推的方向是推点指向箱子、乘 `WalkSpeed`；
    /// 主角走到推点，清掉箱子压住的格子，转状态 1，扣一次次数，登记 `ObjLockInput`。
    fn push_box_touch<W: World>(&mut self, i: usize, ev: u8, w: &mut W) {
        if ev != 1 || self.objs[i].state != 0 {
            return;
        }
        let Some((xlen, zlen)) = w.object_extent(i) else {
            return;
        };
        let c = w.object_pos(i);
        let off = BOX_ROLE_OFFSET[w.controlled_role().clamp(0, 4) as usize];
        let points = [
            (c.x - (xlen + off), c.z),
            (c.x + (xlen + off), c.z),
            (c.x, c.z - (zlen + off)),
            (c.x, c.z + (zlen + off)),
        ];
        let pl = w.player_pos();
        let (mut best, mut dir) = (100000_f32, 0);
        for (k, (x, z)) in points.iter().enumerate() {
            let d = (pl.x - x).hypot(pl.z - z);
            if d < best {
                (best, dir) = (d, k);
            }
        }
        let (x, z) = points[dir];
        let to = Vec3::new(x, pl.y, z);
        let layer = self.objs[i].layer;
        if best >= TOUCH_DIST || !w.can_role_goto(i, layer, &pl, &to) {
            w.hint("outdist");
            return;
        }
        let (dx, dz) = (c.x - x, c.z - z);
        let len = dx.hypot(dz).max(1e-6);
        let speed = w.npc_walk_speed();
        let o = &mut self.objs[i];
        o.push_dir = dir;
        o.push_vel = Vec3::new(dx / len * speed, 0., dz / len * speed);
        self.push(
            i,
            Act::Walk {
                to,
                check: false,
                fix: true,
            },
            w,
        );
        self.run(i, w);
        w.object_tiles(i, true, layer);
        self.objs[i].state = 1;
        self.use_once(i, w);
        self.objs[i].locked = true;
    }

    /// `FUN_0042bfe0`（`IsPushAllow`）：三态箱子状态 0 时只能沿 z 推（方向 2、3），状态 1 只能往 +z、
    /// 状态 2 只能往 −z。
    fn push_allowed(&self, i: usize) -> bool {
        let o = &self.objs[i];
        match o.box_state {
            0 => o.push_dir == 2 || o.push_dir == 3,
            1 => o.push_dir == 2,
            2 => o.push_dir == 3,
            _ => true,
        }
    }

    /// `FUN_00426eb0` case 0xb。状态 1：主角还在往推点走就等着；走到了转状态 2、站住、换成推的动作、
    /// 朝推的方向，记下箱子这时的位置、计时清零，放音效。状态 2：三态箱子（参数 0 为 2）这个方向推不动的，
    /// 推满 1 秒就收尾；否则按经过的时间挪（最多 `50 / WalkSpeed` 秒，即一格 50），主角跟着平移；挪不过去
    /// 收尾；推满收尾并跑自己那条过程。
    fn step_push_box<W: World>(&mut self, i: usize, dt: f32, w: &mut W) {
        match self.objs[i].state {
            1 if self.objs[i].queue.is_empty() => {
                let o = &mut self.objs[i];
                o.state = 2;
                o.timer = 0.;
                let vel = o.push_vel;
                w.stand();
                w.set_act(ACT_PUSH);
                let pl = w.player_pos();
                w.face_player(&Vec3::new(pl.x + vel.x, pl.y, pl.z + vel.z));
                self.objs[i].push_from = w.object_pos(i);
                self.sound(i, 0, w);
            }
            2 => {
                self.objs[i].timer += dt;
                let t = self.objs[i].timer;
                if self.objs[i].p[0] == 2 && !self.push_allowed(i) {
                    if t > 1. {
                        self.finish_box(i, w);
                    }
                    return;
                }
                let dur = BOX_STEP / w.npc_walk_speed();
                let over = dur < t;
                let t = if over { dur } else { t };
                self.objs[i].timer = t;
                let (from, vel) = (self.objs[i].push_from, self.objs[i].push_vel);
                let at = w.object_pos(i);
                let pl = w.player_pos();
                let (ox, oz) = (pl.x - at.x, pl.z - at.z);
                let next = Vec3::new(from.x + vel.x * t, from.y + vel.y * t, from.z + vel.z * t);
                if !w.box_can_go(i, &next) {
                    self.finish_box(i, w);
                    return;
                }
                w.set_object_pos(i, &next);
                w.carry_player(&Vec3::new(next.x + ox, pl.y, next.z + oz));
                if over {
                    self.finish_box(i, w);
                    self.own_proc(i, w);
                }
            }
            _ => {}
        }
    }

    /// `FUN_0042afa0`（`FinishBox`）：按箱子现在的位置登记格子；主角站在箱子压住的格子里的，按推的方向
    /// 挪到那块格子外头紧挨着的一格；回状态 0、主角站住。三态箱子这一推推得动的，按 [`BOX_NEXT_STATE`]
    /// 换状态，参数 1、2 那两扇升降机关门照新状态开关（0 两扇都关，1 开参数 1 那扇，2 开参数 2 那扇）。
    /// 原位 +0xd4 改成现在的位置（移植端挪箱子时已经记进存档）。
    fn finish_box<W: World>(&mut self, i: usize, w: &mut W) {
        let layer = self.objs[i].layer;
        w.object_tiles(i, false, layer);
        let pl_layer = w.player_layer();
        if let Some([x0, z0, x1, z1]) = w.object_owned_rect(i, pl_layer) {
            let (mut x, mut z) = w.player_tile(pl_layer);
            if x0 <= x && x <= x1 && z0 <= z && z <= z1 {
                match self.objs[i].push_dir {
                    0 => x = x0 - 1,
                    1 => x = x1 + 1,
                    2 => z = z0 - 1,
                    _ => z = z1 + 1,
                }
                let (p, _) = w.tile_point(pl_layer, x, z);
                w.carry_player(&p);
            }
        }
        self.objs[i].state = 0;
        w.stand();
        if self.objs[i].p[0] == 2 && self.push_allowed(i) {
            let o = &mut self.objs[i];
            let next = BOX_NEXT_STATE[o.push_dir - 2][o.box_state.min(2) as usize];
            o.box_state = next;
            w.save_state(i, next);
            let (pz, pnz) = (o.p[1], o.p[2]);
            let (z, nz) = (self.index_of(pz), self.index_of(pnz));
            match next {
                0 => {
                    if let Some(z) = z {
                        self.door_switch(z, false, w);
                    }
                    if let Some(nz) = nz {
                        self.door_switch(nz, false, w);
                    }
                }
                1 => {
                    if let Some(z) = z {
                        self.door_switch(z, true, w);
                    }
                }
                2 => {
                    if let Some(nz) = nz {
                        self.door_switch(nz, true, w);
                    }
                }
                _ => {}
            }
        }
    }

    /// `FUN_0042c030`（`Door_Switch`）：升降机关门的开关标志（+0xaf）换了才动：开了转状态 1（往下沉）、
    /// 关了转状态 2（升回来），计时清零，记下现在的位置，升降高度取模型高 − 2，「可通过」跟着标志，放音效。
    fn door_switch<W: World>(&mut self, i: usize, on: bool, w: &mut W) {
        if w.switch_on(i) == on {
            return;
        }
        w.set_switch(i, on);
        let o = &mut self.objs[i];
        o.state = if on { 1 } else { 2 };
        o.timer = 0.;
        o.push_from = w.object_pos(i);
        o.push_vel = Vec3::new(w.object_model_height(i) - 2., 0., 0.);
        w.set_passable(i, on);
        self.sound(i, 0, w);
    }

    /// `FUN_00426eb0` case 0x1e：状态 1 两秒沉下「升降高度」、到了转 3；状态 2 两秒升回、到了转 0。
    fn step_door<W: World>(&mut self, i: usize, dt: f32, w: &mut W) {
        let state = self.objs[i].state;
        if state != 1 && state != 2 {
            return;
        }
        let o = &mut self.objs[i];
        o.timer += dt;
        if DOOR_SECS <= o.timer {
            o.timer = DOOR_SECS;
            o.state = if state == 1 { 3 } else { 0 };
        }
        let k = o.push_vel.x * o.timer * 0.5;
        let mut p = o.push_from;
        p.y += if state == 1 { -k } else { k };
        w.set_object_pos(i, &p);
    }

    /// 滑道 `i` 的路线终点（探路用：站上滑道就被带到那儿）。
    pub fn slide_end(&self, i: usize) -> Option<Vec3> {
        self.objs.get(i).and_then(|o| o.path.last().copied())
    }

    /// 参数 0 为 1 的移动浮板（`FUN_00424c20` case 5，`+0xf8 == 1` 那一支）：主角踩上来（事件 0）、闲着才动。
    /// 推的方向 = 浮板位置 − 主角位置（浮板高出 40 以上不理），去掉 y、归一；排「清掉格子、主角走到浮板
    /// 中间、状态 11」，开始跑队列，登记 `ObjLockInput`，再跑自己那条过程。
    fn hop_touch<W: World>(&mut self, i: usize, ev: u8, w: &mut W) {
        if ev != 0 || !self.idle(i) {
            return;
        }
        let at = w.object_pos(i);
        let pl = w.player_pos();
        if at.y - pl.y > PLATFORM_MAX_RISE {
            return;
        }
        let (dx, dz) = (at.x - pl.x, at.z - pl.z);
        let len = dx.hypot(dz).max(1e-6);
        self.objs[i].push_vel = Vec3::new(dx / len, 0., dz / len);
        for a in [
            Act::Tiles(true),
            Act::Walk {
                to: at,
                check: false,
                fix: true,
            },
            Act::State { state: 11, wait: 0 },
        ] {
            self.push(i, a, w);
        }
        self.run(i, w);
        self.objs[i].locked = true;
        self.own_proc(i, w);
    }

    /// `FUN_00426eb0` case 5（参数 0 为 1）。状态 11：现在停在折线第 `box_state` 个点；在末点只能往回、在第 0
    /// 点只能往前，中间的看前后两段（去掉 y、归一）哪段和「推的方向」点积大就往哪走；转状态 12，方向改成那一段
    /// （带 y、归一），时长 = 段长 / 90。状态 12：按时间在两点间插值，主角站在浮板上跟着走、朝那一段；到了换到
    /// 下一个点，往前看 144：那格能走就回状态 0，主角走过去、按新位置登记格子；不能走就回状态 11 接着走。
    fn step_hop<W: World>(&mut self, i: usize, dt: f32, w: &mut W) {
        let n = self.objs[i].path.len();
        if n < 2 {
            return;
        }
        let norm = |v: Vec3| {
            let l = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
            if l > 0. {
                Vec3::new(v.x / l, v.y / l, v.z / l)
            } else {
                v
            }
        };
        match self.objs[i].state {
            11 => {
                let o = &mut self.objs[i];
                let cur = (o.box_state as usize).min(n - 1);
                let dir: isize = if cur == n - 1 {
                    -1
                } else if cur == 0 {
                    1
                } else {
                    let flat = |a: Vec3, b: Vec3| norm(Vec3::new(b.x - a.x, 0., b.z - a.z));
                    let pos = flat(o.path[cur], o.path[cur + 1]);
                    let neg = flat(o.path[cur], o.path[cur - 1]);
                    let v = o.push_vel;
                    let dot = |a: Vec3| a.x * v.x + a.y * v.y + a.z * v.z;
                    if dot(pos) < dot(neg) { -1 } else { 1 }
                };
                let next = (cur as isize + dir) as usize;
                let (a, b) = (o.path[cur], o.path[next]);
                o.push_dir = next;
                o.box_state = cur as u8;
                o.push_vel = norm(Vec3::new(b.x - a.x, b.y - a.y, b.z - a.z));
                let d = Vec3::new(a.x - b.x, a.y - b.y, a.z - b.z);
                o.beam_to = (d.x * d.x + d.y * d.y + d.z * d.z).sqrt() / HOP_SPEED;
                o.timer = 0.;
                o.state = 12;
            }
            12 => {
                let o = &mut self.objs[i];
                o.timer += dt;
                let dur = o.beam_to;
                let end = dur <= o.timer;
                if end {
                    o.timer = dur;
                }
                let k = if dur > 0. { o.timer / dur } else { 1. };
                let (a, b) = (o.path[o.box_state as usize], o.path[o.push_dir]);
                let p = Vec3::new(
                    a.x + (b.x - a.x) * k,
                    a.y + (b.y - a.y) * k,
                    a.z + (b.z - a.z) * k,
                );
                let vel = o.push_vel;
                w.set_object_pos(i, &p);
                w.put_player(&p);
                w.face_player(&Vec3::new(p.x + vel.x, p.y, p.z + vel.z));
                if !end {
                    return;
                }
                let o = &mut self.objs[i];
                o.box_state = o.push_dir as u8;
                let st = o.box_state;
                w.save_state(i, st);
                let dest = Vec3::new(
                    p.x + vel.x * HOP_LOOKAHEAD,
                    p.y + vel.y * HOP_LOOKAHEAD,
                    p.z + vel.z * HOP_LOOKAHEAD,
                );
                let layer = self.objs[i].layer;
                if w.walkable_at(layer, &dest) {
                    self.objs[i].state = 0;
                    for a in [
                        Act::Walk {
                            to: dest,
                            check: false,
                            fix: true,
                        },
                        Act::Tiles(false),
                    ] {
                        self.push(i, a, w);
                    }
                    self.run(i, w);
                } else {
                    self.objs[i].state = 11;
                }
            }
            _ => {}
        }
    }

    /// 读档：三态箱子的状态。
    pub fn set_box_state(&mut self, i: usize, state: u8) {
        if let Some(o) = self.objs.get_mut(i) {
            o.box_state = state;
        }
    }

    fn step_lift<W: World>(&mut self, i: usize, dt: f32, w: &mut W) {
        let state = self.objs[i].state;
        if state != 1 && state != 3 {
            return;
        }
        let o = &self.objs[i];
        let (top, rises_layer) = (o.p[0] as f32, o.p[1] != 0);
        let mut at = w.object_pos(i);
        let mut stopped = None;
        if state == 1 {
            at.y += 30. * dt;
            if top <= at.y {
                at.y = top;
                stopped = Some(2);
            }
        } else {
            at.y -= 30. * dt;
            if at.y <= o.home_y {
                at.y = o.home_y;
                stopped = Some(0);
            }
        }
        w.set_object_pos(i, &at);
        if let Some(s) = stopped {
            self.lift_stop(i, s, rises_layer, w);
            for a in [Act::Delay(2.)] {
                self.push(i, a, w);
            }
            self.run(i, w);
        }
        self.lift_carry(i, w);
        let now = self.objs[i].state;
        if (state == 1 && now == 2) || (state == 3 && now == 0) {
            self.objs[i].lift = false;
            if let Some(l) = self.objs[i].link {
                self.event(l, 2, false, 0.001, w);
            }
            self.own_proc(i, w);
        }
    }

    /// 升降平台停在 `state`（2 上 / 0 下）：可通过，参数 1 非 0 时换层（2 → 1 层、0 → 0 层，载着的主角
    /// 一起换），状态和位置进存档。
    fn lift_stop<W: World>(&mut self, i: usize, state: u8, move_layer: bool, w: &mut W) {
        self.objs[i].state = state;
        w.set_passable(i, true);
        if move_layer {
            let (to, from) = if state == 2 { (1, 0) } else { (0, 1) };
            if self.objs[i].lift {
                w.set_player_layer(to);
            }
            self.objs[i].layer = to;
            w.object_tiles(i, true, from);
            w.object_tiles(i, false, to);
        }
        w.save_state(i, state);
        w.save_pos(i);
    }

    /// `FUN_0042b350`（`MoveTouchedObj`）。
    fn lift_carry<W: World>(&mut self, i: usize, w: &mut W) {
        if let Some((scene, id)) = self.objs[i].depend.clone() {
            if w.depend_switch_on(&scene, id) {
                let mut at = w.object_pos(i);
                at.y = self.objs[i].p[0] as f32;
                w.set_object_pos(i, &at);
                let move_layer = self.objs[i].p[1] != 0;
                self.lift_stop(i, 2, move_layer, w);
            }
        }
        let touched = self.index_of(self.objs[i].p[2]);
        if let Some(t) = touched.filter(|&t| t != i) {
            let y = w.object_pos(i).y;
            let mut tp = w.object_pos(t);
            tp.y = y;
            w.set_object_pos(t, &tp);
            let layer = self.objs[i].layer;
            if self.objs[t].layer != layer {
                let old = self.objs[t].layer;
                w.object_tiles(t, true, old);
                w.object_tiles(t, false, layer);
                self.objs[t].layer = layer;
            }
        }
    }

    /// `FUN_00424c20` case 5，参数 0 为 0 的移动浮板（不看事件）：开关标志开着、板停在两头（状态 0 / 3）、
    /// 板比主角高不过 40 时，记下要往哪头走（在起点就往终点、反之亦然），载人，主角走到板中心（状态 4），
    /// 扣一次次数、登记 `ObjLockInput`。[未做] 参数 0 为 1 的那种（站上去按主角来的方向推着走）。
    fn platform_touch<W: World>(&mut self, i: usize, w: &mut W) {
        let state = self.objs[i].state;
        let Some(plat) = self.objs[i].plat.as_mut() else {
            return;
        };
        if !plat.on || !(state == 0 || state == 3) {
            return;
        }
        let at = w.object_pos(i);
        if at.y - w.player_pos().y > PLATFORM_MAX_RISE {
            return;
        }
        if state == 0 {
            plat.next = 1;
            plat.dist = 0.;
        } else {
            plat.next = 2;
            plat.dist = plat.path.total();
        }
        plat.lift = true;
        self.objs[i].state = 4;
        w.walk_begin(&at);
        self.use_once(i, w);
        self.objs[i].locked = true;
    }

    /// 开关拨完、连着的是移动浮板（`FUN_00426eb0` case 0x17 末尾）：浮板的开关标志跟着开关走；开了就清掉
    /// 它压住的格子、不载人地自己往另一头跑（停在起点往终点，否则往起点）。
    fn platform_push<W: World>(&mut self, l: usize, on: bool, w: &mut W) {
        w.set_switch(l, on);
        let state = self.objs[l].state;
        let layer = self.objs[l].layer;
        let Some(plat) = self.objs[l].plat.as_mut() else {
            return;
        };
        plat.on = on;
        if !on {
            return;
        }
        plat.lift = false;
        if state == 0 {
            plat.next = 1;
            plat.dist = 0.;
            self.objs[l].state = 1;
        } else {
            plat.next = 2;
            plat.dist = plat.path.total();
            self.objs[l].state = 2;
        }
        w.object_tiles(l, true, layer);
    }

    /// `FUN_00426eb0` case 5。
    fn step_platform<W: World>(&mut self, i: usize, dt: f32, w: &mut W) {
        let layer = self.objs[i].layer;
        let state = self.objs[i].state;
        let Some(plat) = self.objs[i].plat.as_mut() else {
            return;
        };
        match state {
            1 | 2 => {
                let forward = state == 1;
                plat.dist += if forward { dt } else { -dt } * PLATFORM_SPEED;
                let total = plat.path.total();
                let arrived = if forward {
                    plat.dist > total
                } else {
                    plat.dist < 0.
                };
                if !arrived {
                    let (p, n) = plat.path.at(plat.dist);
                    w.set_object_pos(i, &p);
                    if plat.lift {
                        let n = if forward {
                            n
                        } else {
                            Vec3::scalar_mul(-1., &n)
                        };
                        w.carry_player(&p);
                        w.face_player(&Vec3::add(&p, &n));
                    }
                    return;
                }
                plat.dist = if forward { total } else { 0. };
                plat.next = if forward { 3 } else { 0 };
                let (p, n) = plat.path.at(plat.dist);
                let lift = plat.lift;
                self.objs[i].state = 5;
                w.set_object_pos(i, &p);
                // 原位 +0xd4 改成这一头（存档里记的就是它，读档后板子停在这一头、状态又是 0）。
                w.save_pos(i);
                w.object_tiles(i, false, layer);
                if !lift {
                    return;
                }
                // 下板：主角沿路线方向（回程反过来）往外 96；那一格走不了就原路载回去。
                let out = Vec3::scalar_mul(
                    if forward {
                        PLATFORM_STEP_OFF
                    } else {
                        -PLATFORM_STEP_OFF
                    },
                    &n,
                );
                let out = Vec3::add(&w.player_pos(), &out);
                if w.object_tile_walkable(i, &out) {
                    w.walk_begin(&out);
                } else {
                    let plat = self.objs[i].plat.as_mut().unwrap();
                    plat.next = if forward { 2 } else { 1 };
                    self.objs[i].state = plat.next;
                }
            }
            3 => {
                if let Some(p) = plat.path.last() {
                    w.set_object_pos(i, &p);
                }
            }
            4 | 5 => {
                if !w.walk_step(dt, false, true) {
                    return;
                }
                let next = plat.next;
                w.stand();
                if state == 4 {
                    w.object_tiles(i, true, layer);
                    let at = w.object_pos(i);
                    w.carry_player(&at);
                }
                self.objs[i].state = next;
            }
            _ => {}
        }
    }

    /// `FUN_00424c20` case 0xc（主角特技机关）：只认调查（事件 1）。正操作的主角得是参数 0 那个，否则提示
    /// 「cantopen」。物件状态 0、主角离它（三维）不到 [`TOUCH_DIST`] 且走得过去时：离得（x、z）不到
    /// 3 × [`ROLE_RADIUS`] 就当场施展（状态 2、主角播「技能」），否则先走到机关**另一侧**——主角到机关的方向
    /// 反过来乘 12（龙葵、参数 0 为 2 时乘模型半径）——再转状态 1。都扣一次次数、登记 `ObjLockInput`。
    /// 够不着提示「outdist」。之后见 [`Self::step`]。
    fn skill_gate<W: World>(&mut self, i: usize, ev: u8, w: &mut W) {
        if ev != 1 {
            return;
        }
        let p0 = self.objs[i].p[0];
        if w.controlled_role() != p0 {
            w.hint("cantopen");
            return;
        }
        if self.objs[i].state != 0 {
            return;
        }
        let obj = w.object_pos(i);
        let pl = w.player_pos();
        let layer = self.objs[i].layer;
        if Vec3::sub(&obj, &pl).norm() >= TOUCH_DIST || !w.can_role_goto(i, layer, &pl, &obj) {
            w.hint("outdist");
            return;
        }
        self.objs[i].timer = 0.;
        let (dx, dz) = (pl.x - obj.x, pl.z - obj.z);
        let len = (dx * dx + dz * dz).sqrt();
        if 1. <= len - ROLE_RADIUS * 3. {
            let k = if p0 == 2 {
                -w.object_extent(i)
                    .map_or(0., |(x, z)| 0.5 * (x * x + z * z).sqrt())
            } else {
                -ROLE_RADIUS
            };
            let to = Vec3::new(obj.x + dx / len * k, obj.y, obj.z + dz / len * k);
            self.push(
                i,
                Act::Walk {
                    to,
                    check: true,
                    fix: true,
                },
                w,
            );
            self.push(i, Act::State { state: 1, wait: 0 }, w);
            self.run(i, w);
        } else {
            self.objs[i].state = 2;
            w.show_act(ACT_SKILL);
        }
        self.use_once(i, w);
        self.objs[i].locked = true;
    }

    /// `FUN_00424c20` case 0x17（开关）。参数 1 定谁能拨：0 只认调查、1 只认别的机关（事件 2）、2 调查时
    /// 只跑自己那条过程（同层、离得不到 200、走得过去，过程没在跑；否则提示「outdist」），其余都认。
    ///
    /// 能拨、状态 0 时先登记 `ObjLockInput`。事件 2：翻开关、状态 3、模型从头按 2 倍速播并放音效、扣次数，
    /// 参数 0 为 1 的「可通过」跟着开关走。调查：取主角→开关连线上离开关半边长（包围盒 x、z 边长大的一半）
    /// 那一点，离主角 [`TOUCH_DIST`] 以上不理；主角离开关（x、z）减半边长不到 [`SWITCH_REACH`] 就当场拨
    /// （状态 2、转身、播「调查」、`FUN_0042c810` 翻开关），否则排「走过去 → 动作 0x13 → 状态 0x4d」。
    fn switch<W: World>(&mut self, i: usize, ev: u8, w: &mut W) {
        let p = self.objs[i].p;
        let obj = w.object_pos(i);
        let pl = w.player_pos();
        let half = self.half_size(i, w);
        let d = Vec3::sub(&obj, &pl).norm();
        match p[1] {
            0 if ev != 1 => return,
            1 if ev != 2 => return,
            2 if ev == 1 => {
                let layer = self.objs[i].layer;
                if w.player_layer() == layer && d < 200. && w.can_role_goto(i, layer, &pl, &obj) {
                    if !w.proc_running() {
                        self.own_proc(i, w);
                    }
                } else {
                    w.hint("outdist");
                }
                return;
            }
            2 if ev != 2 => return,
            _ => {}
        }
        if self.objs[i].state != 0 {
            return;
        }
        self.objs[i].locked = true;
        if ev == 2 {
            let on = w.toggle_switch(i);
            self.objs[i].state = 3;
            if w.object_animated(i) {
                w.play_anim(i, ANIM_SPEED);
                self.sound(i, 0, w);
            }
            self.use_once(i, w);
            if p[0] == 1 {
                w.set_passable(i, on);
            }
            return;
        }
        let f = d - half;
        if TOUCH_DIST <= f || d <= 0. {
            return;
        }
        let to = Vec3::new(
            pl.x + (obj.x - pl.x) / d * f,
            pl.y + (obj.y - pl.y) / d * f,
            pl.z + (obj.z - pl.z) / d * f,
        );
        self.objs[i].timer = 0.;
        let (dx, dz) = (pl.x - obj.x, pl.z - obj.z);
        if (dx * dx + dz * dz).sqrt() - half - SWITCH_REACH < 0. {
            self.objs[i].state = 2;
            w.face_player(&obj);
            w.show_act(ACT_CHECK);
            self.open_switch(i, w);
            return;
        }
        for a in [
            Act::Walk {
                to,
                check: true,
                fix: true,
            },
            Act::DistRestrict,
            Act::State {
                state: SWITCH_PENDING,
                wait: 0,
            },
        ] {
            self.push(i, a, w);
        }
        self.run(i, w);
    }

    /// 包围盒 x、z 边长大的那个的一半（开关、动作 0x13 用，`FUN_00429ee0` 取的旋转后的盒子，边长不变）。
    fn half_size<W: World>(&self, _i: usize, w: &W) -> f32 {
        w.object_extent(_i).map_or(0., |(x, z)| x.max(z) * 0.5)
    }

    /// 次数不是无限就扣一次。
    fn use_once<W: World>(&mut self, i: usize, w: &mut W) {
        let o = &mut self.objs[i];
        if o.times != 0xff {
            o.times = o.times.saturating_sub(1);
            let t = o.times;
            w.save_times(i, t);
        }
    }

    /// `FUN_0042c810`（`SceneObj::OpenSwitch`）：扣次数、翻开关。[未做] 参数 2 为 1（神树花的总开关）时
    /// 还要把场景里所有类型 0x25 的花一起开合。
    /// `FUN_0042c810`：扣一次次数、翻开关；参数 2 为 1 的是神树花总开关，花开没开 = 开关是不是 0，
    /// 场上的花跟着换（`FUN_0042c280`）。
    fn open_switch<W: World>(&mut self, i: usize, w: &mut W) {
        self.use_once(i, w);
        let on = w.toggle_switch(i);
        if self.objs[i].p[2] == 1 {
            w.switch_flowers(!on);
        }
    }

    fn sound<W: World>(&self, i: usize, p: i32, w: &mut W) {
        let o = &self.objs[i];
        if let Some(name) = object_sound(o.sound.as_deref(), o.kind, o.p[0], p) {
            let name = name.to_string();
            w.sound(&name);
        }
    }

    /// `FUN_0042af80`：主角从物件 `i` 上离开。类型 9 状态回 0，0x26 计时清零。
    pub fn leave(&mut self, i: usize) {
        if let Some(o) = self.objs.get_mut(i) {
            match o.kind {
                9 => o.state = 0,
                0x26 => o.timer = 0.,
                _ => {}
            }
        }
    }

    /// `FUN_00426eb0` 里和队列、状态有关的部分，每帧对每个在场的物件调。
    pub fn step<W: World>(&mut self, dt: f32, w: &mut W) {
        for i in 0..self.objs.len() {
            if !w.active(i) {
                continue;
            }
            self.advance(i, dt, w);
            match self.objs[i].kind {
                0x5 if self.objs[i].p[0] == 1 => self.step_hop(i, dt, w),
                0x5 => self.step_platform(i, dt, w),
                7 | 8 => self.step_beam(i, dt, w),
                0xb => self.step_push_box(i, dt, w),
                0x1e => self.step_door(i, dt, w),
                // `FUN_00426eb0` case 0x24：状态 1 时清掉触发区、转 2。
                0x24 if self.objs[i].state == 1 => {
                    w.clear_trigger_rect(i);
                    self.objs[i].state = 2;
                    w.save_state(i, 2);
                }
                0x16 => self.step_lift(i, dt, w),
                0xc => self.step_skill_gate(i, w),
                0x17 => self.step_switch(i, dt, w),
                0x1c => self.step_hammer(i, dt, w),
                0x20 => self.step_flower(i, w),
                _ => {}
            }
            let o = &mut self.objs[i];
            let mut changed = None;
            if o.kind == 0x1d {
                if o.state == 1 {
                    o.state = 2;
                    o.timer = 0.;
                    changed = Some(2);
                } else if o.state == 2 {
                    o.timer += dt;
                    if o.timer > TURTLE_SETTLE_SECS {
                        o.state = 3;
                        changed = Some(3);
                    }
                }
            }
            if let Some(state) = changed {
                w.save_state(i, state);
            }
        }
        // `ObjLockInput::Update`：不在场、或者闲下来的物件除名。
        for i in 0..self.objs.len() {
            if self.objs[i].locked && (!w.active(i) || self.is_static(i)) {
                self.objs[i].locked = false;
            }
        }
    }

    /// `FUN_00426eb0` case 0xc。状态 1（走到了）：转向机关、站住、播「技能」，转 2。2：主角动作播完，
    /// 机关模型按 2 倍速播、放音效，转 3。3：模型播完（不是动画模型就当场算完），回 0、清掉压住的格子、
    /// 物件不在场。
    fn step_skill_gate<W: World>(&mut self, i: usize, w: &mut W) {
        match self.objs[i].state {
            1 => {
                let p = w.object_pos(i);
                w.face_player(&p);
                w.stand();
                self.objs[i].timer = 0.;
                self.objs[i].state = 2;
                w.show_act(ACT_SKILL);
            }
            2 => {
                if !w.player_acting() {
                    w.stand();
                    self.objs[i].state = 3;
                    if w.object_animated(i) {
                        w.play_anim(i, ANIM_SPEED);
                    }
                    self.sound(i, 0, w);
                }
            }
            3 => {
                if !w.object_animated(i) || w.anim_done(i) {
                    self.objs[i].state = 0;
                    let layer = self.objs[i].layer;
                    w.object_tiles(i, true, layer);
                    w.deactivate(i);
                }
            }
            _ => {}
        }
    }

    /// `FUN_00426eb0` case 0x20（食人花）。状态 1：主角飞完，花的模型按 2 倍速播（吞）、主角不在场，转 2。
    /// 2：模型播完，停回第一帧，主角回来、状态 0；主角放到参数 (0, 1) 那格，渲染位置挪到参数 2 那朵
    /// （配对的）花上，再飞向那格的中心（y 取格子自己的高度）。
    fn step_flower<W: World>(&mut self, i: usize, w: &mut W) {
        match self.objs[i].state {
            1 => {
                if !w.player_acting() {
                    w.play_anim(i, ANIM_SPEED);
                    self.objs[i].state = 2;
                    w.set_player_shown(false);
                }
            }
            2 => {
                if w.object_animated(i) && !w.anim_done(i) {
                    return;
                }
                w.reset_anim(i);
                w.set_player_shown(true);
                self.objs[i].state = 0;
                let (p, layer) = (self.objs[i].p, self.objs[i].layer);
                let at = self
                    .index_of(p[2])
                    .map_or_else(|| w.object_pos(i), |o| w.object_pos(o));
                w.place_player(layer, p[0], p[1], &at);
                let (center, _) = w.tile_point(layer, p[0], p[1]);
                w.fly_player(&center, 2);
            }
            _ => {}
        }
    }

    /// `FUN_00426eb0` case 0x1c（剑冢的剑或锤）。计时 +0x54 加 dt。参数 0 为 0 的剑左右摆：绕 Z 转
    /// 24·sin t 度。别的参数、开关开着的上下动：y = 原位 + 参数 1 ×（sin t + 1）/ 2，不低于脚下地面
    /// （`FUN_00421440`）。参数 0 不为 1、或者开着的，离上次受伤超过 1.2 秒、碰撞盒碰到主角：全队扣最大精的
    /// 15%（参数 0 为 1）/ 10%（别的）。
    fn step_hammer<W: World>(&mut self, i: usize, dt: f32, w: &mut W) {
        self.objs[i].timer += dt;
        let t = self.objs[i].timer;
        let p0 = self.objs[i].p[0];
        let on = w.switch_on(i);
        if p0 == 0 {
            w.set_object_roll(i, 24. * t.sin());
        } else if on {
            let o = &self.objs[i];
            let mut p = w.object_pos(i);
            p.y = o.p[1] as f32 * (t.sin() + 1.) * 0.5 + o.home_y;
            let floor = w.floor_at(o.layer, &p);
            if p.y < floor {
                p.y = floor;
            }
            w.set_render_pos(i, &p);
        }
        if (p0 != 1 || on) && w.hurt_ready() && w.player_hit_by(i) {
            w.obj_hurt(if p0 == 1 { 0.15 } else { 0.1 });
        }
    }

    /// `FUN_00426eb0` case 0x17。0x4d（走到了）：`FUN_0042c810` 翻开关，转 1。1：站住、转向开关、播「调查」，
    /// 转 2。2：主角动作播完，开关模型从头按 2 倍速播、放音效，转 3。3：模型播完（不是动画模型就当场算完），
    /// 停在最后一帧，回 0，跑自己那条过程（登记 `ObjLockInput`）；参数 0 为 1 的「可通过」跟着开关走；
    /// 连着的物件收到事件 2。11：世界转动（参数 2 为 2）。
    ///
    /// [未做] 参数 2 为 1（神树花总开关，`FUN_0042c730`）、参数 2 为 2 的世界转动（状态 11）、连着的是
    /// 移动浮板（类型 5）时让浮板开到另一头——这三样都只记日志。
    fn step_switch<W: World>(&mut self, i: usize, dt: f32, w: &mut W) {
        match self.objs[i].state {
            SWITCH_PENDING => {
                self.open_switch(i, w);
                self.objs[i].state = 1;
            }
            1 => {
                w.stand();
                self.objs[i].timer = 0.;
                self.objs[i].state = 2;
                let p = w.object_pos(i);
                w.face_player(&p);
                w.show_act(ACT_CHECK);
            }
            2 => {
                if !w.player_acting() {
                    w.stand();
                    self.objs[i].state = 3;
                    w.play_anim(i, ANIM_SPEED);
                    self.sound(i, 0, w);
                }
            }
            11 => {
                self.objs[i].timer += dt;
                if self.objs[i].timer >= ROTATE_SECS {
                    self.objs[i].state = 0;
                }
            }
            3 => {
                if w.object_animated(i) && !w.anim_done(i) {
                    return;
                }
                self.objs[i].state = 0;
                self.own_proc(i, w);
                let p = self.objs[i].p;
                if p[0] == 1 {
                    // `+0xd1 = +0xaf`：开关这时已经翻过。
                    let on = w.switch_on(i);
                    w.set_passable(i, on);
                }
                if p[2] == 2 {
                    log::warn!("场景物件：#{i} 要转动世界，没做");
                }
                if let Some(l) = self.objs[i].link {
                    if self.objs[l].kind == 5 {
                        let on = w.switch_on(i);
                        self.platform_push(l, on, w);
                    } else {
                        self.event(l, 2, false, 0.001, w);
                    }
                }
            }
            _ => {}
        }
    }

    /// 往队尾加一条（`FUN_0042c950` 等：起点记排队时物件的位置）。
    fn push<W: World>(&mut self, i: usize, act: Act, w: &W) {
        let dur = match act {
            Act::Move { dur, .. } => dur,
            Act::Delay(secs) => secs,
            _ => 0.,
        };
        let from = w.object_pos(i);
        self.objs[i].queue.push_back(Entry {
            act,
            dur,
            elapsed: 0.,
            from,
        });
    }

    /// `FUN_0042ebe0`：从队头做起，当场做完的出队、接着做，要等的停下。
    fn run<W: World>(&mut self, i: usize, w: &mut W) {
        while let Some(e) = self.objs[i].queue.front() {
            match e.act.clone() {
                Act::Move { .. } | Act::Delay(_) => return,
                Act::PlayerPath { .. } => {
                    w.stand();
                    let mut points = vec![w.player_pos()];
                    points.extend(self.objs[i].path.iter().copied());
                    let path = PathLine::new(points);
                    let dur = path.total() / SLIDE_SPEED;
                    if let Some(e) = self.objs[i].queue.front_mut() {
                        e.act = Act::PlayerPath { path: Some(path) };
                        e.dur = dur;
                        e.elapsed = 0.;
                    }
                    w.set_act(ACT_SLIDE);
                    return;
                }
                Act::Fall { .. } => {
                    let layer = self.objs[i].layer;
                    let floor = w.nav_floor(layer, &w.object_pos(i)) - FALL_BELOW_FLOOR;
                    if let Some(e) = self.objs[i].queue.front_mut() {
                        e.act = Act::Fall {
                            to_y: floor,
                            vy: 0.,
                        };
                    }
                    return;
                }
                Act::Walk { to, .. } => {
                    w.walk_begin(&to);
                    return;
                }
                Act::State { state, wait } => {
                    let o = &mut self.objs[i];
                    o.state = state;
                    o.timer = 0.;
                    // 开关（0x17）在存档里那一格存的是开关标志（126 `GetSwitch` 读它），不能拿状态盖掉——
                    // 走过去再拨时排的「状态 0x4d」先写进去，紧接着翻开关就读成了「开着」、翻成了关。
                    // 特技机关、食人花、踏板这几种的状态只是过场，读档回来不用。
                    if !matches!(o.kind, 0xc | 0x17 | 0x20 | 0x21) {
                        w.save_state(i, state);
                    }
                    if wait != 0 {
                        return;
                    }
                }
                Act::Link { wait } => {
                    // 没连着就什么都不做（`LinkObject` 对 `0xFFFF` 直接返回 NULL）。
                    // 物件自己那条过程不在这里跑，见 [`ObjectActions::own_proc`]。
                    if let Some(l) = self.objs[i].link {
                        self.event(l, 2, false, 0.001, w);
                    }
                    if wait {
                        return;
                    }
                }
                Act::Anim { .. } => {
                    w.play_anim(i, ANIM_SPEED);
                    return;
                }
                Act::Proc { id, wait } => {
                    // 过程号 0 是没挂过程（`FUN_00432170(0, 0)` 找不到，什么也不跑）。
                    if id != 0 {
                        w.call_proc(id);
                    }
                    if wait {
                        return;
                    }
                }
                Act::Tiles(clear) => {
                    let layer = self.objs[i].layer;
                    w.object_tiles(i, clear, layer);
                }
                Act::Layer { layer, object } => {
                    w.set_player_layer(layer);
                    if object {
                        self.objs[i].layer = layer;
                    }
                }
                Act::Freeze(release) => w.freeze(release),
                Act::Passable(v) => w.set_passable(i, v),
                Act::Face(p) => w.face_player(&p),
                Act::Sound(p) => self.sound(i, p, w),
                Act::DistRestrict => {
                    // `FUN_0042ebe0` case 0x13：y 分量是 −主角 +8，而主角 +8 是相对地面的高度，站着恒 0，
                    // 所以只量 x、z。
                    let o = w.object_pos(i);
                    let pl = w.player_pos();
                    let d = Vec3::new(o.x - pl.x, 0., o.z - pl.z).norm();
                    if d - self.half_size(i, w) > DIST_RESTRICT {
                        self.objs[i].queue.clear();
                        w.hint("outdist");
                        return;
                    }
                }
            }
            self.objs[i].queue.pop_front();
        }
    }

    /// `FUN_0042f460`：推队头那条一帧，到了就出队、接着 [`Self::run`]。
    fn advance<W: World>(&mut self, i: usize, dt: f32, w: &mut W) {
        let kind = self.objs[i].kind;
        let state = self.objs[i].state;
        let link_idle = self.objs[i].link.map(|l| self.idle(l));
        let Some(e) = self.objs[i].queue.front_mut() else {
            return;
        };
        e.elapsed += dt;
        let mut done = e.dur <= e.elapsed;
        if done {
            e.elapsed = e.dur;
        }
        match e.act.clone() {
            Act::Move { to, carry, .. } => {
                let k = if e.dur > 0. {
                    (1. - (e.elapsed / e.dur * std::f32::consts::PI).cos()) * 0.5
                } else {
                    1.
                };
                let p = Vec3::new(
                    to.x * k + (1. - k) * e.from.x,
                    to.y * k + (1. - k) * e.from.y,
                    to.z * k + (1. - k) * e.from.z,
                );
                w.set_object_pos(i, &p);
                if carry {
                    w.carry_player(&p);
                }
                if kind == 0x21 || kind == 0x1d {
                    w.fix_player_height();
                }
            }
            Act::PlayerPath { path: Some(path) } => {
                let k = if e.dur > 0. {
                    (1. - (e.elapsed / e.dur * std::f32::consts::PI).cos()) * 0.5
                } else {
                    1.
                };
                let (p, n) = path.at(k * path.total());
                w.carry_player(&p);
                w.face_player(&Vec3::new(p.x + n.x, p.y, p.z + n.z));
                if done {
                    w.stand();
                }
            }
            Act::Fall { to_y, vy } => {
                let vy = vy - dt * FALL_GRAVITY;
                let mut p = w.object_pos(i);
                p.y += vy * dt;
                if to_y < p.y {
                    e.act = Act::Fall { to_y, vy };
                    w.set_object_pos(i, &p);
                    return;
                }
                p.y = to_y;
                w.set_object_pos(i, &p);
                w.save_pos(i);
                done = true;
            }
            Act::Walk { check, fix, .. } => {
                if !w.walk_step(dt, check, fix) {
                    return;
                }
                done = true;
            }
            Act::State { wait, .. } => match wait {
                1 => done = state != 1,
                2 => done = state == 0,
                _ => {}
            },
            // 没有连着的物件就当场算完。
            Act::Link { .. } => done = link_idle.unwrap_or(true),
            // 等过程跑完（`FUN_004324b0`）。[移植] 只能看「有没有脚本在跑」，认不出是不是自己起的那条。
            Act::Proc { wait: true, .. } => done = !w.proc_running(),
            // `hold`：原版把时间倍率置 0、跳到最后一帧；移植端单播本来就停在最后一帧。
            Act::Anim { .. } => {
                if !w.anim_done(i) {
                    return;
                }
                done = true;
            }
            _ => {}
        }
        if done {
            self.objs[i].queue.pop_front();
            self.run(i, w);
        }
    }
}

/// `FUN_0042c440`：按物件类型挑音效。`p0` 是物件参数 0，`p` 是动作带的参数。
/// 特技机关按参数 0（哪个主角）分五种（`We026`..`We030`），开关（0x17）表里没有。
fn sound_name(kind: u8, p0: i32, p: i32) -> Option<&'static str> {
    match kind {
        0xc => match p0 {
            0 => Some("We026"),
            1 => Some("We027"),
            2 => Some("We028"),
            3 => Some("We029"),
            4 => Some("We030"),
            _ => None,
        },
        0x13 => Some("wg001"),
        0x1d => Some(if p == 0 { "wg005" } else { "we026" }),
        0x27 => Some("wc014"),
        7 => Some("wg004"),
        0xb => Some("We025"),
        0x1e => Some("Wg005"),
        0x16 => Some("Wg005"),
        _ => None,
    }
}

/// `FUN_0042c440` 开头：物件自己带名字（+0x234 非空）就用它，否则按类型查表。
fn object_sound<'a>(own: Option<&'a str>, kind: u8, p0: i32, p: i32) -> Option<&'a str> {
    own.or_else(|| sound_name(kind, p0, p))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Mock {
        /// `switch_flowers` 收到的花开没开。
        flowers: Vec<bool>,
        pos: Vec<Vec3>,
        player: Vec3,
        party: usize,
        freeze: i32,
        sounds: Vec<String>,
        hints: Vec<String>,
        passable: Vec<bool>,
        /// 每个物件动画还剩几帧播完。
        anim_left: Vec<i32>,
        played: Vec<usize>,
        walking: Option<(Vec3, i32)>,
        faced: Option<Vec3>,
        height_fixes: usize,
        procs: Vec<i32>,
        tiles: Vec<(usize, bool, usize)>,
        player_layer: usize,
        saved_states: Vec<(usize, u8)>,
        proc_busy: bool,
        controlled: i32,
        switches: Vec<bool>,
        /// 主角动作还剩几帧播完。
        acting: i32,
        acts: Vec<usize>,
        deactivated: Vec<usize>,
        saved_times: Vec<(usize, u8)>,
        extent: Option<(f32, f32)>,
        /// 各物件绕 Y 的角度，和存过几次角度。
        yaw: Vec<f32>,
        saved_yaws: usize,
        /// 清过几次触发区。
        trigger_clears: usize,
        /// `walkable_at`：x 不到这个值才算能走。
        walkable_max_x: f32,
        /// 各物件绕 Z 的角度；离上次受伤够不够久、碰撞盒碰没碰到主角、扣过的比例。
        roll: Vec<f32>,
        hurt_ready: bool,
        hit: bool,
        hurts: Vec<f32>,
    }

    impl World for Mock {
        fn active(&self, _: usize) -> bool {
            true
        }
        fn object_pos(&self, i: usize) -> Vec3 {
            self.pos[i]
        }
        fn set_object_pos(&mut self, i: usize, p: &Vec3) {
            self.pos[i] = *p;
        }
        fn player_pos(&self) -> Vec3 {
            self.player
        }
        fn carry_player(&mut self, p: &Vec3) {
            self.player = *p;
        }
        fn fix_player_height(&mut self) {
            self.height_fixes += 1;
        }
        fn play_anim(&mut self, i: usize, speed: f32) {
            assert_eq!(speed, 2.);
            self.played.push(i);
            self.anim_left[i] = 3;
        }
        fn anim_done(&self, i: usize) -> bool {
            self.anim_left[i] <= 0
        }
        fn freeze(&mut self, release: bool) {
            self.freeze += if release { 1 } else { -1 };
        }
        fn set_passable(&mut self, i: usize, v: bool) {
            self.passable[i] = v;
        }
        fn face_player(&mut self, p: &Vec3) {
            self.faced = Some(*p);
        }
        fn walk_begin(&mut self, to: &Vec3) {
            self.walking = Some((*to, 2));
        }
        fn walk_step(&mut self, _: f32, _: bool, _: bool) -> bool {
            match self.walking.as_mut() {
                Some((to, n)) if *n > 0 => {
                    *n -= 1;
                    if *n == 0 {
                        self.player = *to;
                    }
                    false
                }
                _ => true,
            }
        }
        fn sound(&mut self, name: &str) {
            self.sounds.push(name.into());
        }
        fn party_size(&self) -> usize {
            self.party
        }
        fn proc_running(&self) -> bool {
            self.proc_busy
        }
        fn call_proc(&mut self, id: i32) {
            self.procs.push(id);
        }
        fn object_tiles(&mut self, i: usize, clear: bool, layer: usize) {
            self.tiles.push((i, clear, layer));
        }
        fn object_yaw(&self, i: usize) -> f32 {
            self.yaw[i]
        }
        fn set_object_yaw(&mut self, i: usize, deg: f32) {
            self.yaw[i] = deg;
        }
        fn save_yaw(&mut self, _i: usize) {
            self.saved_yaws += 1;
        }
        fn nav_floor(&self, _layer: usize, _p: &Vec3) -> f32 {
            100.
        }
        fn clear_trigger_rect(&mut self, _i: usize) {
            self.trigger_clears += 1;
        }
        fn walkable_at(&self, _layer: usize, p: &Vec3) -> bool {
            p.x < self.walkable_max_x
        }
        fn set_player_layer(&mut self, layer: usize) {
            self.player_layer = layer;
        }
        fn object_model_height(&self, _: usize) -> f32 {
            10.
        }
        fn tile_point(&self, layer: usize, x: i32, z: i32) -> (Vec3, f32) {
            (
                Vec3::new(x as f32 * 12. + 6., 50. + layer as f32, z as f32 * 12. + 6.),
                500. + layer as f32,
            )
        }
        fn switch_flowers(&mut self, open: bool) {
            self.flowers.push(open);
        }
        fn object_rect(&self, _: usize) -> [i32; 4] {
            [44, 88, 46, 92]
        }
        fn floor_at(&self, layer: usize, _: &Vec3) -> f32 {
            layer as f32 * 1700. - 1670.
        }
        fn hint(&mut self, key: &str) {
            self.hints.push(key.into());
        }
        fn save_state(&mut self, i: usize, state: u8) {
            self.saved_states.push((i, state));
        }
        fn controlled_role(&self) -> i32 {
            self.controlled
        }
        fn object_extent(&self, _: usize) -> Option<(f32, f32)> {
            self.extent
        }
        fn show_act(&mut self, act: usize) {
            self.acts.push(act);
            self.acting = 2;
        }
        fn player_acting(&self) -> bool {
            self.acting > 0
        }
        fn toggle_switch(&mut self, i: usize) -> bool {
            self.switches[i] = !self.switches[i];
            self.switches[i]
        }
        fn switch_on(&self, i: usize) -> bool {
            self.switches[i]
        }
        fn set_switch(&mut self, i: usize, on: bool) {
            self.switches[i] = on;
        }
        fn set_object_roll(&mut self, i: usize, deg: f32) {
            self.roll[i] = deg;
        }
        fn hurt_ready(&self) -> bool {
            self.hurt_ready
        }
        fn player_hit_by(&self, _: usize) -> bool {
            self.hit
        }
        fn obj_hurt(&mut self, frac: f32) {
            self.hurts.push(frac);
            self.hurt_ready = false;
        }
        fn deactivate(&mut self, i: usize) {
            self.deactivated.push(i);
        }
        fn save_times(&mut self, i: usize, t: u8) {
            self.saved_times.push((i, t));
        }
    }

    impl Mock {
        fn frame(&mut self, a: &mut ObjectActions, dt: f32) {
            for n in self.anim_left.iter_mut() {
                *n -= 1;
            }
            self.acting -= 1;
            a.step(dt, self);
        }
    }

    /// M01/1：OBJECT_10 乌龟（参数 0 = 3，连着 0xb）、OBJECT_11 吊桥。
    fn m01() -> (ObjectActions, Mock) {
        let a = ObjectActions::new(&[
            ObjNode {
                object_id: 10,
                kind: 0x1d,
                layer: 0,
                p: [3, 0, 0, 0],
                link_id: 0xb,
                sound: None,
                proc: 0,
                times: 0xff,
                examinable: false,
                path: vec![],
                home_y: 0.,
                depend: None,
                switch_init: false,
            },
            ObjNode {
                object_id: 11,
                kind: 0x13,
                layer: 0,
                p: [0, 0, 0, 0],
                link_id: 0xffff,
                sound: None,
                proc: 0,
                times: 0xff,
                examinable: false,
                path: vec![],
                home_y: 0.,
                depend: None,
                switch_init: false,
            },
        ]);
        let w = Mock {
            pos: vec![Vec3::new(475., 123., -1030.), Vec3::new(369., 130., -1158.)],
            player: Vec3::new(475. + 20., 147., -1030.),
            party: 3,
            freeze: 1,
            sounds: vec![],
            hints: vec![],
            passable: vec![true, false],
            anim_left: vec![0, 0],
            played: vec![],
            walking: None,
            faced: None,
            height_fixes: 0,
            procs: vec![],
            tiles: vec![],
            player_layer: 0,
            saved_states: vec![],
            proc_busy: false,
            controlled: 0,
            switches: vec![false, false],
            acting: 0,
            acts: vec![],
            deactivated: vec![],
            saved_times: vec![],
            extent: None,
            yaw: vec![0.; 4],
            saved_yaws: 0,
            trigger_clears: 0,
            walkable_max_x: f32::MAX,
            roll: vec![0.; 2],
            hurt_ready: true,
            hit: false,
            hurts: vec![],
            flowers: vec![],
        };
        (a, w)
    }

    #[test]
    fn turtle_needs_enough_party_members() {
        let (mut a, mut w) = m01();
        w.party = 2;
        a.event(0, 0, false, 0.1, &mut w);
        a.event(0, 0, true, 0.1, &mut w);
        assert_eq!(w.hints, ["lessweight"]);
        assert!(a.idle(0));
        assert_eq!(w.freeze, 1);
    }

    #[test]
    fn turtle_ignores_player_beyond_reach() {
        let (mut a, mut w) = m01();
        w.player = Vec3::new(475. + 36.5, 147., -1030.);
        a.event(0, 0, false, 0.1, &mut w);
        assert!(a.idle(0));
        w.player = Vec3::new(475. + 36., 400., -1030.);
        a.event(0, 0, false, 0.1, &mut w);
        assert!(!a.idle(0), "高度不算进距离，正好 36 也算");
    }

    #[test]
    fn turtle_sinks_and_lowers_the_bridge() {
        let (mut a, mut w) = m01();
        a.event(0, 0, false, 0.1, &mut w);
        // 冻住、开始走。
        assert_eq!(w.freeze, 0);
        assert!(w.walking.is_some());
        assert!(w.sounds.is_empty());
        // 再站上来也不排（队列不空）。
        a.event(0, 0, true, 0.1, &mut w);
        assert_eq!(w.freeze, 0);

        let dt = 0.125;
        w.frame(&mut a, dt);
        w.frame(&mut a, dt);
        assert!(w.sounds.is_empty());
        // 第三帧走完：We026、开始播乌龟的动画。
        w.frame(&mut a, dt);
        assert_eq!(w.sounds, ["we026"]);
        assert_eq!(w.played, [0]);
        // 动画播完那帧：出队、置状态 1、Wg005，同一帧里状态机接着把 1 变成 2；沉从下一帧开始推。
        for _ in 0..3 {
            w.frame(&mut a, dt);
        }
        assert_eq!(w.sounds, ["we026", "wg005"]);
        assert_eq!(a.state(0), Some(2));
        assert_eq!(w.pos[0].y, 123.);
        w.frame(&mut a, dt);
        let y0 = w.pos[0].y;
        assert!(y0 < 123. && y0 > 122.9, "余弦缓动起步很慢：{y0}");
        // 沉到 1.25 秒正好一半。
        for _ in 0..9 {
            w.frame(&mut a, dt);
        }
        assert!((w.pos[0].y - 119.5).abs() < 1e-3, "{}", w.pos[0].y);
        assert!(w.height_fixes > 0);
        for _ in 0..10 {
            w.frame(&mut a, dt);
        }
        // 沉完：叫吊桥。吊桥冻住、主角转过去、Wg001、播吊桥动画；乌龟接着解冻。
        assert_eq!(w.pos[0].y, 116.);
        assert_eq!(w.sounds, ["we026", "wg005", "wg001"]);
        let f = w.faced.unwrap();
        assert_eq!((f.x, f.y, f.z), (369., 130., -1158.));
        assert_eq!(w.played, [0, 1]);
        assert_eq!(w.freeze, 0, "乌龟解冻了、吊桥还冻着");
        assert!(!w.passable[1]);
        for _ in 0..3 {
            w.frame(&mut a, dt);
        }
        assert!(w.passable[1]);
        assert_eq!(w.freeze, 1);
        assert_eq!(a.state(1), Some(1));
        // 乌龟状态 2 过 2.5 秒变 3。
        assert_eq!(a.state(0), Some(3));
        assert!(!a.busy());
        // 以后都不再动。
        a.event(0, 0, false, 0.1, &mut w);
        a.event(1, 2, false, 0.1, &mut w);
        assert!(!a.busy());
    }

    #[test]
    fn turtle_without_a_link_runs_its_own_proc() {
        // M24/2 的乌龟：没连着别的物件，挂着过程 506（对五个巨锤各来一次「与物件交互」）。
        let mut a = ObjectActions::new(&[ObjNode {
            object_id: 10,
            kind: 0x1d,
            layer: 0,
            p: [4, 0, 0, 0],
            link_id: 0xffff,
            sound: None,
            proc: 506,
            times: 0xff,
            examinable: false,
            path: vec![],
            home_y: 0.,
            depend: None,
            switch_init: false,
        }]);
        let (_, mut w) = m01();
        w.pos = vec![Vec3::new(0., 100., 0.)];
        w.player = Vec3::new(10., 100., 0.);
        w.party = 4;
        w.passable = vec![true];
        w.anim_left = vec![0];
        a.event(0, 0, false, 0.1, &mut w);
        let dt = 0.125;
        for _ in 0..60 {
            w.frame(&mut a, dt);
        }
        assert_eq!(w.procs, [506]);
        assert_eq!(a.state(0), Some(3));
    }

    #[test]
    fn object_sound_prefers_its_own_name() {
        // 物件自己带名字就用它（M01/1 的吊桥记录里写的是 wg007，不是类型表的 wg001）。
        assert_eq!(object_sound(Some("wg007"), 0x13, 0, 0), Some("wg007"));
        assert_eq!(object_sound(None, 0x13, 0, 0), Some("wg001"));
        assert_eq!(object_sound(None, 0x1d, 0, 0), Some("wg005"));
        assert_eq!(object_sound(None, 0x1d, 0, 1), Some("we026"));
        assert_eq!(object_sound(None, 0x11, 0, 0), None);
        assert_eq!(object_sound(None, 0xc, 3, 0), Some("We029"));
        assert_eq!(object_sound(None, 0x17, 0, 0), None);
    }

    #[test]
    fn link_matches_the_whole_word_like_the_exe() {
        let a = ObjectActions::new(&[
            ObjNode {
                object_id: 4,
                kind: 0x1d,
                layer: 0,
                p: [4, 0, 0, 0],
                link_id: 0x5,
                sound: None,
                proc: 0,
                times: 0xff,
                examinable: false,
                path: vec![],
                home_y: 0.,
                depend: None,
                switch_init: false,
            },
            ObjNode {
                object_id: 5,
                kind: 0x1d,
                layer: 0,
                p: [4, 0, 0, 0],
                link_id: 0xffff,
                sound: None,
                proc: 0,
                times: 0xff,
                examinable: false,
                path: vec![],
                home_y: 0.,
                depend: None,
                switch_init: false,
            },
            // 高字节非零：`cmp bx, di` 配不上任何物件（实机数据里没有这种）。
            ObjNode {
                object_id: 6,
                kind: 0x1d,
                layer: 0,
                p: [4, 0, 0, 0],
                link_id: 0x104,
                sound: None,
                proc: 0,
                times: 0xff,
                examinable: false,
                path: vec![],
                home_y: 0.,
                depend: None,
                switch_init: false,
            },
        ]);
        assert_eq!(a.objs[0].link, Some(1));
        assert_eq!(a.objs[1].link, None);
        assert_eq!(a.objs[2].link, None);
        assert_eq!(a.index_of(5), Some(1));
    }

    /// 一个物件的场景：乌龟那套 m01() 的世界，换成单个 `kind` 物件。
    fn single(kind: u8, p: [i32; 4], layer: usize) -> (ObjectActions, Mock) {
        let (_, mut w) = m01();
        w.pos = vec![Vec3::new(100., 300., 200.)];
        w.player = Vec3::new(100., 300., 200.);
        w.passable = vec![true];
        w.anim_left = vec![0];
        let a = ObjectActions::new(&[ObjNode {
            object_id: 11,
            kind,
            layer,
            p,
            link_id: 0xffff,
            sound: None,
            proc: 0,
            times: 0xff,
            examinable: false,
            path: vec![],
            home_y: 0.,
            depend: None,
            switch_init: false,
        }]);
        (a, w)
    }

    #[test]
    fn portal_needs_four_seconds_standing() {
        let (mut a, mut w) = single(0x26, [101, 0, 0, 0], 0);
        // 刚站上来那帧不算。
        a.event(0, 0, false, 1., &mut w);
        for _ in 0..3 {
            a.event(0, 0, true, 1.25, &mut w);
        }
        assert!(a.idle(0) && w.freeze == 1, "3.75 秒还不够");
        a.leave(0);
        for _ in 0..3 {
            a.event(0, 0, true, 1.25, &mut w);
        }
        assert!(a.idle(0), "离开过计时清零");
        a.event(0, 0, true, 0.25, &mut w);
        assert!(!a.idle(0));
        assert_eq!(w.freeze, 0);
        // 走一下（已经在中间，Mock 走两帧）、升 200 用 3 秒、跑过程 101、解冻。
        let dt = 0.125;
        for _ in 0..3 {
            w.frame(&mut a, dt);
        }
        for _ in 0..24 {
            w.frame(&mut a, dt);
        }
        assert_eq!(w.pos[0].y, 500.);
        assert_eq!(w.player.y, 500., "主角跟着升");
        assert_eq!(w.procs, [101]);
        assert_eq!(w.freeze, 1);
        assert!(a.idle(0));
    }

    #[test]
    fn portal_landing_goes_down_with_param_1() {
        let (mut a, mut w) = single(0x26, [0, 106, 0, 0], 0);
        a.event(0, 0, true, 5., &mut w);
        assert!(a.idle(0), "参数 0 是 0，站着不动");
        a.event(0, 3, false, 0., &mut w);
        for _ in 0..30 {
            w.frame(&mut a, 0.125);
        }
        assert_eq!(w.pos[0].y, 100.);
        assert_eq!(w.procs, [106]);
    }

    #[test]
    fn pedal_moves_to_the_other_layer() {
        let (mut a, mut w) = single(0x28, [47, 46, 3, 15], 0);
        a.event(0, 0, false, 0.1, &mut w);
        assert_eq!(w.freeze, 0);
        let dt = 0.125;
        for _ in 0..3 {
            w.frame(&mut a, dt);
        }
        // 1 层的目标格 (3, 15)：地面 501 + 包围盒高 10，8 秒。
        for _ in 0..64 {
            w.frame(&mut a, dt);
        }
        assert_eq!(w.pos[0].y, 511.);
        assert_eq!(w.player.y, 511.);
        assert_eq!(w.tiles, [(0, true, 0), (0, false, 1)]);
        assert_eq!(w.player_layer, 1);
        // 再走到目标格中心，解冻。
        let (to, _) = w.walking.unwrap();
        assert_eq!((to.x, to.z), (42., 186.));
        for _ in 0..3 {
            w.frame(&mut a, dt);
        }
        assert_eq!(w.freeze, 1);
        assert!(a.idle(0));
        // 板现在在 1 层：再站上去就回 0 层、用参数 (47, 46)。
        a.event(0, 0, false, 0.1, &mut w);
        for _ in 0..70 {
            w.frame(&mut a, dt);
        }
        assert_eq!(w.player_layer, 0);
        assert_eq!(w.pos[0].y, 510.);
    }

    fn push_node(object_id: i32, kind: u8, p: [i32; 4]) -> ObjNode {
        ObjNode {
            object_id,
            kind,
            layer: 0,
            p,
            link_id: 0xffff,
            sound: None,
            proc: 77,
            times: 0xff,
            examinable: true,
            path: vec![],
            home_y: 0.,
            depend: None,
            switch_init: false,
        }
    }

    /// 参数 0 为 1 的移动浮板（海底城）：从 −x 那边踩上去就往 +x 走；到了第 1 个点前方 144 不能落脚，按
    /// 推的方向拐到下一段；到了第 2 个点前方能落脚，停下、主角走过去、按新位置登记格子。
    #[test]
    fn hop_platform_rides_until_there_is_ground_ahead() {
        let mut a = ObjectActions::new(&[ObjNode {
            path: vec![
                Vec3::new(0., 0., 0.),
                Vec3::new(300., 0., 0.),
                Vec3::new(300., 0., 300.),
            ],
            ..push_node(11, 0x5, [1, 0, 0, 0])
        }]);
        let (_, mut w) = single(0x5, [1, 0, 0, 0], 0);
        w.pos = vec![Vec3::new(0., 0., 0.)];
        w.player = Vec3::new(-40., 0., 0.);
        w.walkable_max_x = 400.;
        a.event(0, 0, false, 0.1, &mut w);
        assert_eq!(w.tiles, [(0, true, 0)]);
        let dt = 0.5;
        for _ in 0..3 {
            w.frame(&mut a, dt);
        }
        assert_eq!(a.objs[0].state, 12);
        for _ in 0..8 {
            w.frame(&mut a, dt);
        }
        // 300 / 90 秒到第 1 个点：x = 444 那边不能落脚，拐上第二段。
        assert_eq!(a.objs[0].box_state, 1);
        assert_eq!(a.objs[0].state, 12);
        for _ in 0..8 {
            w.frame(&mut a, dt);
        }
        assert_eq!(a.objs[0].box_state, 2);
        assert_eq!(a.objs[0].state, 0);
        assert_eq!((w.pos[0].x, w.pos[0].z), (300., 300.));
        for _ in 0..4 {
            w.frame(&mut a, dt);
        }
        assert_eq!((w.player.x, w.player.z), (300., 444.));
        assert_eq!(w.tiles, [(0, true, 0), (0, false, 0)]);
        assert!(w.saved_states.contains(&(0, 2)));
    }

    /// 滑道（0x23）：踩上去（事件 0）就沿「主角位置 + 折线」滑，每秒 600、余弦缓动，到了终点再跑自己那条
    /// 过程（等它跑完），之前一直锁着输入。
    #[test]
    fn slide_carries_the_player_along_its_path() {
        let mut a = ObjectActions::new(&[ObjNode {
            path: vec![Vec3::new(100., 0., 300.), Vec3::new(100., 0., 600.)],
            ..push_node(11, 0x23, [0; 4])
        }]);
        let (_, mut w) = single(0x23, [0; 4], 0);
        w.player = Vec3::new(100., 0., 200.);
        a.event(0, 1, false, 0.1, &mut w);
        assert!(a.idle(0), "调查不理");
        a.event(0, 0, false, 0.1, &mut w);
        assert!(a.input_locked());
        let dt = 0.1;
        for _ in 0..3 {
            w.frame(&mut a, dt);
        }
        assert!(w.player.z > 200. && w.player.z < 400., "缓动：起步慢");
        for _ in 0..5 {
            w.frame(&mut a, dt);
        }
        assert_eq!(w.player.z, 600.);
        assert_eq!(w.procs, [77]);
        w.frame(&mut a, dt);
        assert!(!a.input_locked());
    }

    /// 推箱子（0xb）：调查键才推；主角从 −z 那边走到推点（箱子边长 30、景天往里收 12，离箱子 18），
    /// 推 50 / 80 秒挪一格 50，主角跟着挪；推完按新位置登记格子、跑自己那条过程。
    #[test]
    fn push_box_moves_one_step_away_from_the_player() {
        let (mut a, mut w) = single(0xb, [0; 4], 0);
        a.objs[0].proc = 77;
        w.extent = Some((30., 30.));
        w.player = Vec3::new(100., 300., 140.);
        a.event(0, 0, false, 0.1, &mut w);
        assert_eq!(a.objs[0].state, 0, "只认调查");
        a.event(0, 1, false, 0.1, &mut w);
        assert_eq!(a.objs[0].state, 1);
        assert_eq!(w.tiles, [(0, true, 0)]);
        let dt = 0.125;
        for _ in 0..3 {
            w.frame(&mut a, dt);
        }
        assert_eq!((w.player.x, w.player.z), (100., 182.), "走到推点");
        assert_eq!(a.objs[0].state, 2);
        assert_eq!(w.sounds, ["We025"]);
        for _ in 0..6 {
            w.frame(&mut a, dt);
        }
        assert_eq!(a.objs[0].state, 0);
        assert_eq!(w.pos[0].z, 250.);
        assert_eq!(w.player.z, 232.);
        assert_eq!(w.tiles, [(0, true, 0), (0, false, 0)]);
        assert_eq!(w.procs, [77]);
    }

    /// 三态箱子（参数 0 为 2）：状态 0 时横着推不动（推满 1 秒收尾，箱子不挪）；从 −z 往 +z 推完转状态 2，
    /// 参数 2 那扇升降机关门打开、两秒沉下「模型高 − 2」，可通过。
    #[test]
    fn three_state_box_opens_the_matching_door() {
        let mut a = ObjectActions::new(&[
            push_node(11, 0xb, [2, 12, 13, 0]),
            push_node(12, 0x1e, [0; 4]),
            push_node(13, 0x1e, [0; 4]),
        ]);
        let (_, mut w) = single(0xb, [0; 4], 0);
        w.pos = vec![
            Vec3::new(100., 300., 200.),
            Vec3::new(0., 0., 0.),
            Vec3::new(500., 40., 0.),
        ];
        w.passable = vec![false; 3];
        w.anim_left = vec![0; 3];
        w.switches = vec![false; 3];
        w.extent = Some((30., 30.));
        let dt = 0.125;
        // 从 −x 那边推：方向 0，推不动。
        w.player = Vec3::new(40., 300., 200.);
        a.event(0, 1, false, 0.1, &mut w);
        for _ in 0..14 {
            w.frame(&mut a, dt);
        }
        assert_eq!(a.objs[0].state, 0);
        assert_eq!((w.pos[0].x, w.pos[0].z), (100., 200.));
        assert_eq!(a.objs[0].box_state, 0);
        // 从 −z 那边推：方向 2，状态 0 → 2，开参数 2 那扇。
        w.player = Vec3::new(100., 300., 140.);
        a.event(0, 1, false, 0.1, &mut w);
        for _ in 0..10 {
            w.frame(&mut a, dt);
        }
        assert_eq!(a.objs[0].box_state, 2);
        assert!(w.saved_states.contains(&(0, 2)));
        assert_eq!(w.switches, [false, false, true]);
        assert!(w.passable[2]);
        assert_eq!(a.objs[2].state, 1);
        for _ in 0..16 {
            w.frame(&mut a, dt);
        }
        assert_eq!(a.objs[2].state, 3);
        assert_eq!(w.pos[2].y, 32.);
        assert_eq!(w.pos[1].y, 0., "参数 1 那扇不动");
    }

    /// 坠落的障碍物（0x24）：主角走进来就掉到所在格地面下 4（重力 490），触发区清掉，按落下的位置登记格子、
    /// 不可通过，跑自己那条过程；之后停在状态 2，再走进来不理。
    #[test]
    fn falling_obstacle_drops_and_blocks_the_way_back() {
        let (mut a, mut w) = single(0x24, [0; 4], 0);
        a.objs[0].proc = 500;
        a.event(0, 1, false, 0.1, &mut w);
        assert!(a.idle(0), "调查键不理");
        a.event(0, 0, false, 0.1, &mut w);
        assert!(a.input_locked());
        let dt = 0.05;
        // 从 300 掉到 96：每帧先减速度再挪，n 帧掉 490 × dt² × n(n+1)/2，第 18 帧到。
        for _ in 0..17 {
            w.frame(&mut a, dt);
        }
        assert!(w.pos[0].y > 96.);
        assert!(w.tiles.is_empty());
        for _ in 0..3 {
            w.frame(&mut a, dt);
        }
        assert_eq!(w.pos[0].y, 96.);
        assert_eq!(a.objs[0].state, 2);
        assert_eq!(w.trigger_clears, 1);
        assert_eq!(w.tiles, [(0, false, 0)]);
        assert!(!w.passable[0]);
        assert_eq!(w.procs, [500]);
        assert!(!a.input_locked());
        a.event(0, 0, false, 0.1, &mut w);
        assert!(a.objs[0].queue.is_empty(), "掉下来以后不再动");
    }

    /// 剑冢的锤（0x1c，参数 0 为 1）：只认事件 2，翻开关、「可通过」跟着走；开着时 y = 原位 + 参数 1 ×
    /// （sin t + 1）/ 2、不低于地面；碰到主角扣 15%，受伤计时没过不扣；关着不动、不伤人。
    #[test]
    fn hammer_starts_moving_when_called_and_hurts_on_touch() {
        let (mut a, mut w) = single(0x1c, [1, 145, 0, 0], 0);
        w.switches = vec![false];
        w.passable = vec![false];
        a.objs[0].home_y = 118.;
        w.hit = true;
        a.event(0, 0, false, 0.1, &mut w);
        a.event(0, 1, false, 0.1, &mut w);
        assert!(!w.switches[0] && !w.passable[0], "主角碰、调查都不理");
        w.frame(&mut a, 0.1);
        assert_eq!(w.pos[0].y, 300., "关着不动");
        assert!(w.hurts.is_empty(), "关着不伤人");
        a.event(0, 2, false, 0.1, &mut w);
        assert!(w.switches[0] && w.passable[0]);
        assert!(!a.input_locked(), "不登记 ObjLockInput");
        w.frame(&mut a, 0.1);
        let t = a.objs[0].timer;
        assert!((w.pos[0].y - (118. + 145. * (t.sin() + 1.) * 0.5)).abs() < 1e-3);
        assert_eq!(w.hurts, [0.15]);
        w.frame(&mut a, 0.1);
        assert_eq!(w.hurts, [0.15], "受伤计时没过不再扣");
        a.event(0, 2, false, 0.1, &mut w);
        assert!(!w.switches[0] && !w.passable[0], "再叫一次停下、挡路");
    }

    /// 剑冢的剑（0x1c，参数 0 为 0）：绕 Z 左右摆 24·sin t 度，碰到主角扣 10%；事件 2 不理。
    #[test]
    fn sword_swings_and_hurts_without_a_switch() {
        let (mut a, mut w) = single(0x1c, [0; 4], 0);
        w.switches = vec![false];
        w.hit = true;
        a.event(0, 2, false, 0.1, &mut w);
        assert!(!w.switches[0]);
        w.frame(&mut a, 0.5);
        assert!((w.roll[0] - 24. * 0.5f32.sin()).abs() < 1e-4);
        assert_eq!(w.pos[0].y, 300.);
        assert_eq!(w.hurts, [0.1]);
    }

    /// 旋转石梁（7）：只认事件 2；每秒 30 度转 90 度，转的时候格子清掉、停住按新角度登记、角度进存档，
    /// 再等 2 秒才放开输入；转着的时候再叫不理。
    #[test]
    fn beam_turns_ninety_degrees_when_called() {
        let (mut a, mut w) = single(7, [0; 4], 0);
        w.yaw[0] = 45.;
        a.event(0, 0, false, 0.1, &mut w);
        assert_eq!(a.objs[0].state, 0, "主角碰到不转");
        a.event(0, 2, false, 0.1, &mut w);
        assert_eq!(a.objs[0].state, 1);
        assert_eq!(w.tiles, [(0, true, 0)]);
        assert_eq!(w.sounds, ["wg004"]);
        assert!(a.input_locked());
        a.event(0, 2, false, 0.1, &mut w);
        let dt = 0.125;
        for _ in 0..24 {
            w.frame(&mut a, dt);
        }
        assert_eq!(a.objs[0].state, 1, "3 秒才转到 90 度");
        w.frame(&mut a, dt);
        assert_eq!(a.objs[0].state, 0);
        assert_eq!(w.yaw[0], 135.);
        assert_eq!(w.tiles, [(0, true, 0), (0, false, 0)]);
        assert_eq!(w.saved_yaws, 1);
        assert!(a.input_locked(), "还要等 2 秒");
        for _ in 0..17 {
            w.frame(&mut a, dt);
        }
        assert!(!a.input_locked());
    }

    /// 升降平台（0x16）：只认事件 2；每秒 30 升到参数 0，停住可通过、参数 1 非 0 换到 1 层，等 2 秒才放开
    /// 输入；再叫一次降回原位、换回 0 层。
    #[test]
    fn lift_rises_to_param_0_changes_layer_and_waits_two_seconds() {
        let (mut a, mut w) = single(0x16, [60, 1, 0, 0], 0);
        w.pos[0].y = 0.;
        w.passable[0] = false;
        a.objs[0].home_y = 0.;
        a.event(0, 0, false, 0.1, &mut w);
        assert_eq!(a.state(0), Some(0));
        a.event(0, 2, false, 0.1, &mut w);
        assert_eq!(a.state(0), Some(1));
        assert_eq!(w.sounds, ["Wg005"]);
        assert!(a.input_locked());
        for _ in 0..21 {
            w.frame(&mut a, 0.1);
        }
        assert_eq!(a.state(0), Some(2));
        assert_eq!(w.pos[0].y, 60.);
        assert!(w.passable[0]);
        assert_eq!(a.layer(0), Some(1));
        assert!(a.input_locked());
        for _ in 0..21 {
            w.frame(&mut a, 0.1);
        }
        assert!(!a.input_locked());

        a.event(0, 2, false, 0.1, &mut w);
        assert_eq!(a.state(0), Some(3));
        for _ in 0..21 {
            w.frame(&mut a, 0.1);
        }
        assert_eq!(a.state(0), Some(0));
        assert_eq!(w.pos[0].y, 0.);
        assert_eq!(a.layer(0), Some(0));
    }

    #[test]
    fn path_line_maps_distance_to_a_point_and_its_segment() {
        let l = PathLine::new(vec![
            Vec3::new(0., 0., 0.),
            Vec3::new(100., 0., 0.),
            Vec3::new(100., 0., 50.),
        ]);
        assert_eq!(l.total(), 150.);
        let (p, n) = l.at(50.);
        assert_eq!((p.x, p.z, n.x, n.z), (50., 0., 1., 0.));
        let (p, n) = l.at(120.);
        assert_eq!((p.x, p.z, n.x, n.z), (100., 20., 0., 1.));
        let (p, _) = l.at(-1.);
        assert_eq!((p.x, p.z), (0., 0.));
        let (p, _) = l.at(999.);
        assert_eq!((p.x, p.z), (100., 50.));
    }

    /// 移动浮板：开关没开站上去不动；开关一开板子自己跑到终点（停在 3）；站上去载着主角回起点，
    /// 到了主角往外走 96 下板，板停在 0；载人期间锁输入，停下放开。
    #[test]
    fn platform_runs_on_its_switch_and_carries_the_player_back() {
        let (mut a, mut w) = single(5, [0, 0, 0, 0], 0);
        a.objs[0].plat = Some(Platform {
            path: PathLine::new(vec![
                Vec3::new(100., 300., 200.),
                Vec3::new(300., 300., 200.),
            ]),
            dist: 0.,
            next: 0,
            lift: false,
            on: false,
        });
        a.event(0, 0, false, 0.1, &mut w);
        assert_eq!(a.state(0), Some(0));

        a.platform_push(0, true, &mut w);
        assert_eq!(a.state(0), Some(1));
        for _ in 0..25 {
            w.frame(&mut a, 0.1);
        }
        assert_eq!(a.state(0), Some(3));
        assert_eq!(w.pos[0].x, 300.);
        assert!(a.is_static(0));

        // 主角走上板（在终点那头）。
        w.player = Vec3::new(290., 300., 200.);
        a.event(0, 0, false, 0.1, &mut w);
        assert_eq!(a.state(0), Some(4));
        assert!(a.input_locked());
        for _ in 0..5 {
            w.frame(&mut a, 0.1);
        }
        assert_eq!(a.state(0), Some(2));
        for _ in 0..25 {
            w.frame(&mut a, 0.1);
        }
        // 回到起点：主角跟着板子，再往 −x 走 96 下板。
        assert_eq!(w.pos[0].x, 100.);
        for _ in 0..6 {
            w.frame(&mut a, 0.1);
        }
        assert_eq!(a.state(0), Some(0));
        assert!((w.player.x - 4.).abs() < 1e-3, "{:?}", w.player);
        assert!(!a.input_locked());
    }

    /// M15/2 的 #9（类型 0x27，1 层，矩形 (44, 88)-(46, 92)，参数 0 = 0）：站上去电梯先摆到矩形中间那格，
    /// 主角走过去，电梯连主角 8 秒降到 0 层的地面，主角换层、再往 +z 走 80；电梯自己的层不变。
    #[test]
    fn elevator_carries_the_player_to_the_layer_in_param_0() {
        let (mut a, mut w) = single(0x27, [0, 0, 0, 0], 1);
        a.event(0, 0, true, 0.1, &mut w);
        // 中间那格 (45, 90) 的中心，高度取那格的地面。
        let p = w.pos[0];
        assert_eq!((p.x, p.y, p.z), (546., 51., 1086.));
        assert!(a.input_locked());
        let dt = 0.125;
        for _ in 0..3 {
            w.frame(&mut a, dt);
        }
        assert_eq!(w.sounds, ["wc014"]);
        for _ in 0..64 {
            w.frame(&mut a, dt);
        }
        assert_eq!(w.pos[0].y, -1670.);
        assert_eq!(w.player.y, -1670.);
        assert_eq!(w.player_layer, 0);
        assert_eq!(a.layer(0), Some(1));
        let (to, _) = w.walking.unwrap();
        assert_eq!((to.x, to.y, to.z), (546., -1670., 1166.));
        for _ in 0..3 {
            w.frame(&mut a, dt);
        }
        assert!(a.idle(0));
        assert!(!a.input_locked());
    }

    /// M08/3 的 OBJECT_13（类型 0x17，过程 500，次数 1，参数全 0）：离得远先走过去，走到了翻开关、播「调查」，
    /// 开关模型播完跑过程，次数用完再按不理。
    fn m08_switch() -> (ObjectActions, Mock) {
        let (_, mut w) = m01();
        let a = ObjectActions::new(&[ObjNode {
            object_id: 13,
            kind: 0x17,
            layer: 0,
            p: [0, 0, 0, 0],
            link_id: 0xffff,
            sound: None,
            proc: 500,
            times: 1,
            examinable: true,
            path: vec![],
            home_y: 0.,
            depend: None,
            switch_init: false,
        }]);
        w.pos = vec![Vec3::new(411.86, 0., -300.55)];
        w.passable = vec![false];
        w.anim_left = vec![0];
        w.switches = vec![false];
        w.extent = Some((40., 30.));
        (a, w)
    }

    #[test]
    fn switch_walks_over_then_flips_and_runs_its_proc() {
        let (mut a, mut w) = m08_switch();
        w.player = Vec3::new(411.86 + 150., 0., -300.55);
        a.event(0, 1, false, 0.001, &mut w);
        assert!(a.input_locked());
        // 走到主角→开关连线上离开关 20（半边长）的地方。
        let (to, _) = w.walking.unwrap();
        assert!((to.x - (411.86 + 20.)).abs() < 1e-3, "{to:?}");
        assert!(!w.switches[0]);
        let dt = 0.1;
        for _ in 0..3 {
            w.frame(&mut a, dt);
        }
        // 走到了：动作 0x13 放行，状态 0x4d → 翻开关、扣次数。
        assert!(w.switches[0]);
        assert_eq!(w.saved_times, [(0, 0)]);
        for _ in 0..12 {
            w.frame(&mut a, dt);
        }
        assert_eq!(w.acts, [ACT_CHECK]);
        assert_eq!(w.played, [0]);
        assert_eq!(w.procs, [500]);
        assert!(a.idle(0));
        assert!(!a.input_locked());
        // 过场状态（0x4d、1、2、3）不进存档：那一格是开关标志。
        assert!(w.saved_states.is_empty(), "{:?}", w.saved_states);
        // 次数 0：再按什么都不做。
        a.event(0, 1, false, 0.001, &mut w);
        assert!(a.idle(0));
        assert!(w.switches[0]);
        assert!(!a.examinable(0));
    }

    /// 参数 2 为 1 的是神树花总开关：翻成开（1）花就关，再翻回 0 花就开。普通开关不碰花。
    #[test]
    fn flower_master_switch_closes_and_opens_the_flowers() {
        let (mut a, mut w) = m08_switch();
        a.objs[0].p[2] = 1;
        a.objs[0].times = 0xff;
        w.player = Vec3::new(411.86 + 43., 0., -300.55);
        a.event(0, 1, false, 0.001, &mut w);
        assert_eq!(w.flowers, [false]);
        for _ in 0..20 {
            w.frame(&mut a, 0.1);
        }
        assert!(a.idle(0));
        a.event(0, 1, false, 0.001, &mut w);
        assert_eq!(w.flowers, [false, true]);

        let (mut a, mut w) = m08_switch();
        w.player = Vec3::new(411.86 + 43., 0., -300.55);
        a.event(0, 1, false, 0.001, &mut w);
        assert!(w.switches[0]);
        assert!(w.flowers.is_empty());
    }

    #[test]
    fn switch_within_reach_flips_on_the_spot() {
        let (mut a, mut w) = m08_switch();
        // x、z 距离 − 半边长 20 < 24。
        w.player = Vec3::new(411.86 + 43., 0., -300.55);
        a.event(0, 1, false, 0.001, &mut w);
        assert!(w.walking.is_none());
        assert!(w.switches[0]);
        assert_eq!(w.acts, [ACT_CHECK]);
        assert_eq!(a.state(0), Some(2));
    }

    #[test]
    fn switch_walk_ending_too_far_is_cancelled() {
        let (mut a, mut w) = m08_switch();
        w.player = Vec3::new(411.86 + 150., 0., -300.55);
        a.event(0, 1, false, 0.001, &mut w);
        // 走不过去（替身里走完不挪主角）：动作 0x13 量出来离得太远，清空队列、提示。
        w.walking = Some((w.player, 0));
        for _ in 0..3 {
            w.frame(&mut a, 0.1);
        }
        assert_eq!(w.hints, ["outdist"]);
        assert!(!w.switches[0]);
        assert!(a.idle(0));
    }

    #[test]
    fn skill_gate_needs_the_right_hero_and_then_disappears() {
        let (_, mut w) = m01();
        // M08/2 的 OBJECT_9（wz11，参数 0 = 0 景天）。
        let mut a = ObjectActions::new(&[ObjNode {
            object_id: 9,
            kind: 0xc,
            layer: 0,
            p: [0, 0, 0, 0],
            link_id: 0xffff,
            sound: None,
            proc: 0,
            times: 1,
            examinable: true,
            path: vec![],
            home_y: 0.,
            depend: None,
            switch_init: false,
        }]);
        w.pos = vec![Vec3::new(732.85, 0., -531.69)];
        w.player = Vec3::new(732.85 + 30., 0., -531.69);
        w.passable = vec![false];
        w.anim_left = vec![0];
        w.controlled = 1;
        a.event(0, 1, false, 0.001, &mut w);
        assert_eq!(w.hints, ["cantopen"]);
        assert!(a.idle(0));
        w.controlled = 0;
        // 离得不到 36：就地施展。
        a.event(0, 1, false, 0.001, &mut w);
        assert_eq!(w.acts, [ACT_SKILL]);
        assert_eq!(w.saved_times, [(0, 0)]);
        for _ in 0..10 {
            w.frame(&mut a, 0.1);
        }
        assert_eq!(w.sounds, ["We026"]);
        assert_eq!(w.tiles, [(0, true, 0)]);
        assert_eq!(w.deactivated, [0]);
        assert!(a.idle(0));
    }

    #[test]
    fn skill_gate_far_away_walks_to_the_far_side() {
        let (_, mut w) = m01();
        let mut a = ObjectActions::new(&[ObjNode {
            object_id: 9,
            kind: 0xc,
            layer: 0,
            p: [0, 0, 0, 0],
            link_id: 0xffff,
            sound: None,
            proc: 0,
            times: 1,
            examinable: true,
            path: vec![],
            home_y: 0.,
            depend: None,
            switch_init: false,
        }]);
        w.pos = vec![Vec3::new(0., 0., 0.)];
        w.player = Vec3::new(100., 0., 0.);
        w.passable = vec![false];
        w.anim_left = vec![0];
        a.event(0, 1, false, 0.001, &mut w);
        // 主角到机关的方向反过来乘 12：机关另一侧。
        let (to, _) = w.walking.unwrap();
        assert_eq!((to.x, to.z), (-12., 0.));
        for _ in 0..3 {
            w.frame(&mut a, 0.1);
        }
        assert_eq!(w.acts, [ACT_SKILL]);
    }

    #[test]
    fn flower_swallows_and_spits_out_at_the_paired_one() {
        let (_, mut w) = m01();
        // M10/1 的 #18 / #19（_a.cvd，参数 (268, 67)、配对 33 号；(63, 39)、配对 32 号）。
        let mut a = ObjectActions::new(&[
            ObjNode {
                object_id: 32,
                kind: 0x20,
                layer: 0,
                p: [268, 67, 33, 0],
                link_id: 0xffff,
                sound: None,
                proc: 0,
                times: 0xff,
                examinable: false,
                path: vec![],
                home_y: 0.,
                depend: None,
                switch_init: false,
            },
            ObjNode {
                object_id: 33,
                kind: 0x20,
                layer: 0,
                p: [63, 39, 32, 0],
                link_id: 0xffff,
                sound: None,
                proc: 0,
                times: 0xff,
                examinable: false,
                path: vec![],
                home_y: 0.,
                depend: None,
                switch_init: false,
            },
        ]);
        w.pos = vec![
            Vec3::new(-1263., 137., -1254.),
            Vec3::new(1346., 62., -763.),
        ];
        a.event(0, 0, false, 0.1, &mut w);
        assert_eq!(a.state(0), Some(1));
        assert!(a.input_locked());
        // 站着不动的下一帧不再触发。
        a.event(0, 0, true, 0.1, &mut w);
        assert_eq!(a.state(0), Some(1));
        for _ in 0..10 {
            w.frame(&mut a, 0.1);
        }
        assert_eq!(w.played, [0]);
        assert_eq!(a.state(0), Some(0));
        assert!(!a.input_locked());
    }

    #[test]
    fn pressure_plate_sinks_runs_its_proc_and_is_used_up() {
        let (_, mut w) = m01();
        // M11/1 的 9 号踏板（过程 502，次数 1）。
        let mut a = ObjectActions::new(&[ObjNode {
            object_id: 9,
            kind: 0x21,
            layer: 1,
            p: [0, 0, 0, 0],
            link_id: 0xffff,
            sound: None,
            proc: 502,
            times: 1,
            examinable: false,
            path: vec![],
            home_y: 0.,
            depend: None,
            switch_init: false,
        }]);
        w.pos = vec![Vec3::new(-54.8, -634., 3540.4)];
        w.player = Vec3::new(-54.8 + 20., -634., 3540.4);
        w.passable = vec![true];
        w.anim_left = vec![0];
        // 站着不动的那一帧（上一帧也在上面）不理。
        a.event(0, 0, true, 0.1, &mut w);
        assert!(a.idle(0));
        a.event(0, 0, false, 0.1, &mut w);
        assert!(a.input_locked());
        assert_eq!(w.saved_times, [(0, 0)]);
        for _ in 0..30 {
            w.frame(&mut a, 0.1);
        }
        assert!((w.pos[0].y - (-639.)).abs() < 1e-3, "{:?}", w.pos[0]);
        assert_eq!(w.procs, [502]);
        assert!(a.idle(0));
        assert!(!a.input_locked());
        // 用完了：再踩不动。
        a.event(0, 0, false, 0.1, &mut w);
        assert!(a.idle(0));
        assert_eq!(w.procs, [502]);
    }
}
