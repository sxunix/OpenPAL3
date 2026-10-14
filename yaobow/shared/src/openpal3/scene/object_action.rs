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
//! 触发的主角特技机关（0xc）和开关（0x17），古藤林的食人花（0x20），蜀山故道的踏板开关（0x21）。动作里用到主角、模型、音效、脚本、冻结计数的地方都经 [`World`]
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
            0xc => self.skill_gate(i, ev, w),
            0x17 => self.switch(i, ev, w),
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
    fn open_switch<W: World>(&mut self, i: usize, w: &mut W) {
        self.use_once(i, w);
        w.toggle_switch(i);
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
                0xc => self.step_skill_gate(i, w),
                0x17 => self.step_switch(i, dt, w),
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
            if self.objs[i].locked && (!w.active(i) || self.idle(i)) {
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
                if p[2] == 1 {
                    log::warn!("场景物件：#{i} 是神树花总开关，花的开合没做");
                }
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
                        log::warn!("场景物件：#{i} 连着移动浮板 #{l}，浮板开动没做");
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
                Act::Move { .. } => return,
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
