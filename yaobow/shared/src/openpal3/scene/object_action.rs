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
//! 这里做了吊桥（0x13）、乌龟重力机关（0x1d）、神树传送点（0x26）、上下传送板（0x28）。动作里用到主角、模型、
//! 音效、脚本、冻结计数的地方都经 [`World`] 交给调用方。

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
}

#[derive(Debug, Clone)]
struct Obj {
    kind: u8,
    layer: usize,
    p: [i32; 4],
    link: Option<usize>,
    /// +0x50。
    state: u8,
    /// +0x54。
    timer: f32,
    queue: VecDeque<Entry>,
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
    /// `FUN_0040fc70`：队里几个人。
    fn party_size(&self) -> usize;
    /// `FUN_00430570(key)` + `FUN_00450bc0`：弹一句提示。
    fn hint(&mut self, key: &str);
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
                layer: n.layer,
                p: n.p,
                // `FUN_0042bf90` → `FUN_00421560`：按编号找。[推断] 和脚本找物件一样只比低字节。
                link: (n.link_id != 0xffff)
                    .then(|| ids.iter().position(|id| *id == (n.link_id & 0xff) as i32))
                    .flatten(),
                state: 0,
                timer: 0.,
                queue: VecDeque::new(),
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

    /// `FUN_00424c20`：物件 `i` 收到事件 `ev`；`same` 是上一帧主角也站在它上面，`dt` 是这一帧的秒数。
    pub fn event<W: World>(&mut self, i: usize, ev: u8, same: bool, dt: f32, w: &mut W) {
        let Some(o) = self.objs.get(i) else {
            return;
        };
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
            }
            _ => {}
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
                    w.save_state(i, state);
                    if wait != 0 {
                        return;
                    }
                }
                Act::Link { wait } => {
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
                Act::Proc { id, .. } => w.call_proc(id),
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
                Act::Sound(p) => {
                    if let Some(name) = sound_name(self.objs[i].kind, p) {
                        w.sound(name);
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

/// `FUN_0042c440`：按物件类型挑音效。[未做] 物件自己带名字（+0x234 非空）时用它自己的。
fn sound_name(kind: u8, p: i32) -> Option<&'static str> {
    match kind {
        0x13 => Some("wg001"),
        0x1d => Some(if p == 0 { "wg005" } else { "we026" }),
        _ => None,
    }
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
    }

    impl Mock {
        fn frame(&mut self, a: &mut ObjectActions, dt: f32) {
            for n in self.anim_left.iter_mut() {
                *n -= 1;
            }
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
            },
            ObjNode {
                object_id: 11,
                kind: 0x13,
                layer: 0,
                p: [0, 0, 0, 0],
                link_id: 0xffff,
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
    fn link_by_low_byte_and_missing_link() {
        let a = ObjectActions::new(&[
            ObjNode {
                object_id: 4,
                kind: 0x1d,
                layer: 0,
                p: [4, 0, 0, 0],
                link_id: 0x104,
            },
            ObjNode {
                object_id: 5,
                kind: 0x1d,
                layer: 0,
                p: [4, 0, 0, 0],
                link_id: 0xffff,
            },
        ]);
        assert_eq!(a.objs[0].link, Some(0));
        assert_eq!(a.objs[1].link, None);
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
}
