//! The turn loop: drives a [`Battle`] frame by frame, hands the party's
//! turns to a menu and the enemies' turns to the AI, and keeps what the
//! settlement needs. Nothing here knows about the renderer or the script
//! VM — the SCE `Combat` command owns the session and draws it.
//!
//! What the original does at this layer (`FUN_004055d0` case 2 → the
//! per-frame `FUN_004db7a0` / `FUN_004f7910` pair) is: fill gauges, let
//! whoever is ready act, test the outcome after every action. The menu
//! is `FUN_00500d50`. Only the entries that have a rules-side
//! implementation are offered: 攻击 / 仙术 / 防御 / 逃跑. 道具 and 合击
//! are not yet ported and are simply absent from the list — a
//! placeholder, not a claim about the original's menu.

use std::collections::VecDeque;

use super::actions::{ATTR_HP, SkillEffect};
use super::ai::{self, Decision, SkillOption};
use super::formulas::CombatRng;
use super::settlement::{self, DefeatedEnemy, Spoils, Survivor};
use super::state::{Battle, Due, K_CONFUSED, K_SLEEP, Outcome, SLOTS, Side};

/// A skill as the menu offers it.
#[derive(Debug, Clone)]
pub struct SkillChoice {
    pub name: String,
    pub effect: SkillEffect,
}

impl SkillChoice {
    /// A positive 精 impact is a heal and is aimed at one's own side.
    pub fn heals(&self) -> bool {
        self.effect.impacts[ATTR_HP].1 > 0
    }
}

/// What an enemy is worth when it falls (its database row's
/// `experience`, `money_when_killed`, `normal_loot`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Reward {
    pub experience: i32,
    pub money: i32,
    pub loot: Option<(u32, i32)>,
}

/// One combatant's non-rules data: what to call it, what it can cast,
/// what it pays out.
#[derive(Debug, Clone, Default)]
pub struct Profile {
    pub name: String,
    pub skills: Vec<SkillChoice>,
    pub reward: Reward,
}

/// A button press, already mapped from keyboard or pad.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Cancel,
}

/// The top-level menu entries, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuEntry {
    Attack,
    Skill,
    Defend,
    Escape,
}

impl MenuEntry {
    pub const ALL: [MenuEntry; 4] = [
        MenuEntry::Attack,
        MenuEntry::Skill,
        MenuEntry::Defend,
        MenuEntry::Escape,
    ];

    pub fn label(self) -> &'static str {
        match self {
            MenuEntry::Attack => "攻击",
            MenuEntry::Skill => "仙术",
            MenuEntry::Defend => "防御",
            MenuEntry::Escape => "逃跑",
        }
    }
}

/// What the target cursor is choosing a target for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Attack,
    /// Index into the actor's skill list.
    Cast(usize),
}

/// Where the cursor is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    Main {
        index: usize,
    },
    Skills {
        index: usize,
    },
    /// `index` is into [`CombatSession::targets`].
    Target {
        action: Action,
        index: usize,
    },
}

/// A party member waiting for the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerTurn {
    pub slot: usize,
    pub menu: Menu,
}

/// How many log lines are kept for the overlay.
pub const LOG_LINES: usize = 8;

/// How long a transient notice (逃跑失败 and the like) stays up.
const NOTICE_SECONDS: f32 = 1.2;

/// 已经选定、正在出手段等待生效的行动。原版把这个记在行动任务的记录里
/// （`FUN_00514870` 的 `task->+0x2c`/`+0x28`），出手条满了才真正结算。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pending {
    Attack {
        slot: usize,
        target: usize,
    },
    Cast {
        slot: usize,
        target: usize,
        skill: usize,
    },
    Escape {
        slot: usize,
    },
}

impl Pending {
    pub fn slot(&self) -> usize {
        match self {
            Pending::Attack { slot, .. }
            | Pending::Cast { slot, .. }
            | Pending::Escape { slot } => *slot,
        }
    }
}

#[derive(Clone)]
pub struct CombatSession {
    pub battle: Battle,
    profiles: Vec<Profile>,
    /// The AI's own generator, separate from the rules' (`0x4f4...`
    /// reads `DAT_00c11760`, the combat one reads the CRT seed).
    ai_rng: CombatRng,
    pub elapsed: f32,
    pub log: VecDeque<String>,
    pub turn: Option<PlayerTurn>,
    pub notice: Option<(String, f32)>,
    defeated: Vec<DefeatedEnemy>,
    /// Every skill the party cast, `(slot, skill id)`, for the use
    /// counters the settlement levels skills with.
    pub casts: Vec<(usize, u32)>,
    /// 选好了但还在出手段的那个行动，见 [`Pending`]。同一时刻只可能有一个
    /// ——调度器一次只排一个行动任务。
    pub pending: Option<Pending>,
    finished: Option<Outcome>,
}

impl CombatSession {
    /// `profiles` is indexed by battle slot; an unused slot's profile is
    /// ignored.
    pub fn new(battle: Battle, profiles: Vec<Profile>, ai_seed: u32) -> Self {
        let mut profiles = profiles;
        profiles.resize_with(SLOTS, Profile::default);
        Self {
            battle,
            profiles,
            ai_rng: CombatRng::new(ai_seed),
            elapsed: 0.,
            log: VecDeque::new(),
            turn: None,
            notice: None,
            pending: None,
            defeated: Vec::new(),
            casts: Vec::new(),
            finished: None,
        }
    }

    pub fn name(&self, slot: usize) -> &str {
        &self.profiles[slot].name
    }

    pub fn skills(&self, slot: usize) -> &[SkillChoice] {
        &self.profiles[slot].skills
    }

    pub fn finished(&self) -> Option<Outcome> {
        self.finished
    }

    /// Slots on `side` that are still standing, in slot order.
    pub fn standing(&self, side: Side) -> Vec<usize> {
        (0..SLOTS)
            .filter(|s| {
                self.battle
                    .get(*s)
                    .map(|c| c.side == side && c.in_play())
                    .unwrap_or(false)
            })
            .collect()
    }

    /// Who the current target cursor may land on.
    pub fn targets(&self) -> Vec<usize> {
        let Some(PlayerTurn {
            slot,
            menu: Menu::Target { action, .. },
        }) = self.turn
        else {
            return Vec::new();
        };
        let own_side = match action {
            Action::Attack => false,
            Action::Cast(i) => self.skills(slot).get(i).map(|s| s.heals()).unwrap_or(false),
        };
        if own_side {
            self.standing(Side::Party)
        } else {
            self.standing(Side::Enemy)
        }
    }

    /// Whether `slot` can pay for its `i`-th skill right now.
    pub fn can_afford(&self, slot: usize, i: usize) -> bool {
        let (Some(c), Some(s)) = (self.battle.get(slot), self.skills(slot).get(i)) else {
            return false;
        };
        c.sp >= s.effect.sp_cost && c.mp >= s.effect.mp_cost
    }

    /// Advance the battle by `dt` seconds. Returns the outcome once the
    /// battle is over; `None` while it runs or waits for the player.
    pub fn tick(&mut self, dt: f32) -> Option<Outcome> {
        if let Some(o) = self.finished {
            return Some(o);
        }
        if let Some((_, ttl)) = self.notice.as_mut() {
            *ttl -= dt;
            if *ttl <= 0. {
                self.notice = None;
            }
        }
        if self.turn.is_some() {
            return None;
        }
        if self.settle_outcome() {
            return self.finished;
        }

        self.elapsed += dt;
        let Some(due) = self.battle.update(dt) else {
            return None;
        };

        match due {
            // 出手条满了：把先前选定的行动结算掉。
            Due::Executed(slot) => {
                self.resolve_pending(slot);
                self.settle_outcome();
                return self.finished;
            }
            Due::Ready(slot) => {
                let actor = self
                    .battle
                    .get(slot)
                    .expect("update returned a placed slot");
                let side = actor.side;
                let confused = actor.has_state(K_CONFUSED);
                let asleep = actor.has_state(K_SLEEP);

                if asleep {
                    self.push_log(format!("{} 睡着了", self.name(slot)));
                    self.battle.finish_turn(slot);
                } else if side == Side::Party && !confused {
                    self.turn = Some(PlayerTurn {
                        slot,
                        menu: Menu::Main { index: 0 },
                    });
                    return None;
                } else {
                    self.act_by_ai(slot, confused);
                }
            }
        }
        self.settle_outcome();
        self.finished
    }

    /// 出手段结束，行动生效。原版在这一步之后一律走 `FUN_004f4820` →
    /// `FUN_004db710`（两条清零、回蓄力段）。
    fn resolve_pending(&mut self, slot: usize) {
        let Some(p) = self.pending.take() else {
            // 没有登记的行动（例如被打断后又自己走完了出手段），当作空过。
            self.battle.finish_turn(slot);
            return;
        };
        if p.slot() != slot {
            // 不是这个单位的行动，放回去。
            self.pending = Some(p);
            self.battle.finish_turn(slot);
            return;
        }
        match p {
            Pending::Attack { slot, target } => self.attack(slot, target),
            Pending::Cast {
                slot,
                target,
                skill,
            } => self.cast(slot, target, skill),
            Pending::Escape { slot } => {
                let ok = self.battle.try_escape(slot);
                self.push_log(format!(
                    "{} 逃跑{}",
                    self.name(slot),
                    if ok { "成功" } else { "失败" }
                ));
                if !ok {
                    self.notice("逃跑失败");
                }
            }
        }
        self.battle.finish_turn(slot);
    }

    /// 登记一个行动并进入出手段。`multiplier` 是 `FUN_004f76c0` 给的出手速度
    /// 倍率，普通攻击没有技能可查，按 1 走。
    fn commit(&mut self, action: Pending, multiplier: i32) {
        let slot = action.slot();
        self.pending = Some(action);
        match action {
            Pending::Escape { .. } => self.battle.begin_escape(slot),
            _ => self.battle.begin_execute(slot, multiplier),
        }
    }

    /// Let the AI take the pending player turn — 快进 while a script is
    /// being skipped.
    pub fn auto_play(&mut self) {
        let Some(PlayerTurn { slot, .. }) = self.turn.take() else {
            return;
        };
        let confused = self
            .battle
            .get(slot)
            .map(|c| c.has_state(K_CONFUSED))
            .unwrap_or(false);
        self.act_by_ai(slot, confused);
        self.settle_outcome();
    }

    /// Feed one button press into the menu. Ignored when it is not the
    /// player's turn.
    pub fn input(&mut self, input: Input) {
        let Some(mut turn) = self.turn else {
            return;
        };
        let slot = turn.slot;
        match turn.menu {
            Menu::Main { index } => match input {
                Input::Up | Input::Left => {
                    turn.menu = Menu::Main {
                        index: (index + MenuEntry::ALL.len() - 1) % MenuEntry::ALL.len(),
                    };
                }
                Input::Down | Input::Right => {
                    turn.menu = Menu::Main {
                        index: (index + 1) % MenuEntry::ALL.len(),
                    };
                }
                Input::Confirm => match MenuEntry::ALL[index] {
                    MenuEntry::Attack => {
                        turn.menu = Menu::Target {
                            action: Action::Attack,
                            index: 0,
                        };
                    }
                    MenuEntry::Skill => {
                        if self.skills(slot).is_empty() {
                            self.notice("没有可用的仙术");
                        } else {
                            turn.menu = Menu::Skills { index: 0 };
                        }
                    }
                    MenuEntry::Defend => {
                        self.turn = None;
                        self.battle.defend(slot);
                        self.push_log(format!("{} 防御", self.name(slot)));
                        self.battle.finish_turn(slot);
                        self.settle_outcome();
                        return;
                    }
                    MenuEntry::Escape => {
                        // 逃跑也要走一段出手，速度是 速_eff*8+20
                        // （`LAB_00515200` 的 0x515295）。
                        self.turn = None;
                        self.commit(Pending::Escape { slot }, 1);
                        return;
                    }
                },
                Input::Cancel => {}
            },
            Menu::Skills { index } => {
                let n = self.skills(slot).len();
                match input {
                    Input::Up | Input::Left => {
                        turn.menu = Menu::Skills {
                            index: (index + n - 1) % n,
                        };
                    }
                    Input::Down | Input::Right => {
                        turn.menu = Menu::Skills {
                            index: (index + 1) % n,
                        };
                    }
                    Input::Confirm => {
                        if self.can_afford(slot, index) {
                            turn.menu = Menu::Target {
                                action: Action::Cast(index),
                                index: 0,
                            };
                        } else {
                            self.notice("精气不足");
                        }
                    }
                    Input::Cancel => {
                        turn.menu = Menu::Main { index: 1 };
                    }
                }
            }
            Menu::Target { action, index } => {
                let targets = self.targets();
                let n = targets.len().max(1);
                match input {
                    Input::Up | Input::Left => {
                        turn.menu = Menu::Target {
                            action,
                            index: (index + n - 1) % n,
                        };
                    }
                    Input::Down | Input::Right => {
                        turn.menu = Menu::Target {
                            action,
                            index: (index + 1) % n,
                        };
                    }
                    Input::Confirm => {
                        let Some(target) = targets.get(index).copied() else {
                            return;
                        };
                        self.turn = None;
                        match action {
                            Action::Attack => self.commit(Pending::Attack { slot, target }, 1),
                            Action::Cast(i) => {
                                let m = self.cast_multiplier(slot, i);
                                self.commit(
                                    Pending::Cast {
                                        slot,
                                        target,
                                        skill: i,
                                    },
                                    m,
                                )
                            }
                        }
                        return;
                    }
                    Input::Cancel => {
                        turn.menu = match action {
                            Action::Attack => Menu::Main { index: 0 },
                            Action::Cast(i) => Menu::Skills { index: i },
                        };
                    }
                }
            }
        }
        self.turn = Some(turn);
    }

    /// The payout for what has fallen so far — the original settles the
    /// same way after a win, a round-out and a successful escape.
    pub fn spoils(&self) -> Spoils {
        let party: Vec<Survivor> = (0..SLOTS)
            .filter_map(|slot| {
                let c = self.battle.get(slot)?;
                (c.side == Side::Party).then_some(Survivor {
                    slot,
                    alive: c.alive,
                    exp_bonus_percent: 0.,
                    money_bonus_percent: 0.,
                })
            })
            .collect();
        settlement::settle(&self.defeated, &party)
    }

    fn notice(&mut self, text: &str) {
        self.notice = Some((text.to_string(), NOTICE_SECONDS));
    }

    fn push_log(&mut self, line: String) {
        if self.log.len() >= LOG_LINES {
            self.log.pop_front();
        }
        self.log.push_back(line);
    }

    /// Record the outcome once; true when the battle is over.
    fn settle_outcome(&mut self) -> bool {
        if self.finished.is_some() {
            return true;
        }
        let outcome = self.battle.outcome();
        if outcome != Outcome::Ongoing {
            self.finished = Some(outcome);
            self.turn = None;
        }
        outcome != Outcome::Ongoing
    }

    fn act_by_ai(&mut self, slot: usize, confused: bool) {
        let options: Vec<SkillOption> = self
            .skills(slot)
            .iter()
            .enumerate()
            .filter(|(i, _)| self.can_afford(slot, *i))
            .map(|(_, s)| SkillOption {
                id: s.effect.id,
                hp_impact: s.effect.impacts[ATTR_HP].1 as i32,
            })
            .collect();
        match ai::choose(&self.battle, &mut self.ai_rng, slot, &options, confused) {
            Decision::Pass => {
                // 不出手的这一支原版直接走 `FUN_004db750(k,0,0,0)`
                // （0x513ac2），整条清零跳过这次。
                self.push_log(format!("{} 按兵不动", self.name(slot)));
                self.battle.finish_turn(slot);
            }
            Decision::Attack { target } => self.commit(Pending::Attack { slot, target }, 1),
            Decision::Cast { target, skill } => {
                match self.skills(slot).iter().position(|s| s.effect.id == skill) {
                    Some(i) => {
                        let m = self.cast_multiplier(slot, i);
                        self.commit(
                            Pending::Cast {
                                slot,
                                target,
                                skill: i,
                            },
                            m,
                        )
                    }
                    None => self.commit(Pending::Attack { slot, target }, 1),
                }
            }
        }
    }

    /// `FUN_004f76c0` 给的出手速度倍率。原版从角色的技能槽里取一个字节再
    /// 加 1，取不到返回 0（任务里当 1 用）。那个字节在移植的技能数据里还没
    /// 对应上，所以这里一律按 1 `[推断]`——效果是出手段以两倍蓄力速度推进，
    /// 和原版没有该字段时的行为一致。
    fn cast_multiplier(&self, _slot: usize, _skill: usize) -> i32 {
        1
    }

    fn attack(&mut self, slot: usize, target: usize) {
        let moved = self.battle.basic_attack(slot, target);
        self.push_log(format!(
            "{} 攻击 {} 精{:+}",
            self.name(slot),
            self.name(target),
            moved
        ));
        self.note_deaths(slot);
    }

    fn cast(&mut self, slot: usize, target: usize, i: usize) {
        let Some(choice) = self.skills(slot).get(i).cloned() else {
            return;
        };
        if self.battle.get(slot).map(|c| c.side) == Some(Side::Party) {
            self.casts.push((slot, choice.effect.id));
        }
        let r = self.battle.cast_skill(slot, target, &choice.effect);
        let line = if r.hit {
            format!(
                "{} {} → {} 精{:+}",
                self.name(slot),
                choice.name,
                self.name(target),
                r.deltas[ATTR_HP]
            )
        } else {
            format!(
                "{} {} → {} 未命中",
                self.name(slot),
                choice.name,
                self.name(target)
            )
        };
        self.push_log(line);
        self.note_deaths(slot);
    }

    /// Anything that has fallen since the last look gets a settlement
    /// entry, credited to `actor` if it is on the party's side.
    fn note_deaths(&mut self, actor: usize) {
        let killer = self
            .battle
            .get(actor)
            .filter(|c| c.side == Side::Party)
            .map(|_| actor);
        let already = self.defeated.len();
        let fallen: Vec<usize> = (0..SLOTS)
            .filter(|s| {
                self.battle
                    .get(*s)
                    .map(|c| c.side == Side::Enemy && !c.alive)
                    .unwrap_or(false)
            })
            .collect();
        // Enemies do not get up again, so "dead and not yet counted" is
        // exactly the ones beyond what was counted before.
        for slot in fallen.into_iter().skip(already) {
            let reward = self.profiles[slot].reward;
            self.push_log(format!("{} 倒下了", self.name(slot)));
            self.defeated.push(DefeatedEnemy {
                experience: reward.experience,
                money: reward.money,
                loot: reward.loot,
                killer,
                death_time: self.elapsed,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openpal3::combat::actions::ATTRIBUTE_SLOTS;
    use crate::openpal3::combat::state::Combatant;

    fn fighter(side: Side, hp: i32, speed: i32) -> Combatant {
        let mut c = Combatant::new(1, side);
        c.level = 5;
        c.hp = hp;
        c.max_hp = hp;
        c.attack = 100;
        c.defense = 10;
        c.speed = speed;
        c.luck = 10;
        c.max_sp = 50;
        c.sp = 50;
        c
    }

    fn heal() -> SkillChoice {
        let mut impacts = [(0u8, 0i16); ATTRIBUTE_SLOTS];
        impacts[ATTR_HP] = (0, 30);
        SkillChoice {
            name: "治疗".into(),
            effect: SkillEffect {
                id: 1,
                level: 1,
                success_rate_level: 0,
                elements: (None, None),
                impacts,
                sp_cost: 10,
                mp_cost: 0,
            },
        }
    }

    fn session() -> CombatSession {
        let mut b = Battle::new(1);
        // The party member is much faster, so it acts first.
        b.place(0, fighter(Side::Party, 200, 100));
        b.place(5, fighter(Side::Enemy, 60, 1));
        let profiles = vec![
            Profile {
                name: "景天".into(),
                skills: vec![heal()],
                reward: Reward::default(),
            },
            Profile::default(),
            Profile::default(),
            Profile::default(),
            Profile::default(),
            Profile {
                name: "妖".into(),
                skills: vec![],
                reward: Reward {
                    experience: 12,
                    money: 7,
                    loot: Some((3001, 1)),
                },
            },
        ];
        CombatSession::new(b, profiles, 7)
    }

    fn run_until_turn(s: &mut CombatSession) -> usize {
        for _ in 0..10_000 {
            assert!(s.tick(0.05).is_none());
            if let Some(t) = s.turn {
                return t.slot;
            }
        }
        panic!("没人轮到出手");
    }

    #[test]
    fn the_party_gets_a_menu_and_the_enemy_does_not() {
        let mut s = session();
        assert_eq!(run_until_turn(&mut s), 0);
        assert_eq!(
            s.turn.unwrap().menu,
            Menu::Main { index: 0 },
            "菜单从攻击开始"
        );
        // Time stands still while the menu is up.
        let before = s.elapsed;
        s.tick(1.0);
        assert_eq!(s.elapsed, before);
    }

    /// 一直 tick 到战斗有结果——出手段让行动不再是当帧结算。
    fn run_until_finished(s: &mut CombatSession) -> Option<Outcome> {
        for _ in 0..400 {
            if let Some(o) = s.tick(0.05) {
                return Some(o);
            }
            if s.turn.is_some() {
                s.auto_play();
            }
        }
        None
    }

    #[test]
    fn attacking_through_the_menu_settles_a_kill_with_spoils() {
        let mut s = session();
        run_until_turn(&mut s);
        s.input(Input::Confirm); // 攻击
        assert!(matches!(
            s.turn.unwrap().menu,
            Menu::Target {
                action: Action::Attack,
                ..
            }
        ));
        assert_eq!(s.targets(), vec![5]);
        s.input(Input::Confirm);
        assert!(s.turn.is_none());
        // 确认菜单只是**选定**行动，单位这时进的是出手段；要等出手条走满
        // 才真正打出去（原版 `FUN_004db730` → `FUN_004db6c0`）。
        assert!(s.pending.is_some(), "行动应当挂在出手段等着");
        assert_eq!(s.tick(0.05), None, "出手条还没满");
        // 100 attack against 10 defence at level 5 kills a 60 HP enemy.
        assert_eq!(run_until_finished(&mut s), Some(Outcome::Win));
        let spoils = s.spoils();
        assert_eq!(spoils.money, 7);
        assert_eq!(spoils.loot, vec![(3001, 1)]);
        assert_eq!(spoils.experience.len(), 1);
        assert!(spoils.experience[0].1 > 12, "最后一击有三成加成");
    }

    #[test]
    fn a_heal_targets_our_own_side_and_cancel_walks_back() {
        let mut s = session();
        run_until_turn(&mut s);
        s.input(Input::Down); // 仙术
        s.input(Input::Confirm);
        assert_eq!(s.turn.unwrap().menu, Menu::Skills { index: 0 });
        s.input(Input::Confirm);
        assert_eq!(s.targets(), vec![0], "治疗术指向我方");
        s.input(Input::Cancel);
        assert_eq!(s.turn.unwrap().menu, Menu::Skills { index: 0 });
        s.input(Input::Cancel);
        assert_eq!(s.turn.unwrap().menu, Menu::Main { index: 1 });
    }

    #[test]
    fn an_unaffordable_skill_is_refused_with_a_notice() {
        let mut s = session();
        s.battle.get_mut(0).unwrap().sp = 3;
        run_until_turn(&mut s);
        s.input(Input::Down);
        s.input(Input::Confirm);
        s.input(Input::Confirm);
        assert_eq!(s.turn.unwrap().menu, Menu::Skills { index: 0 });
        assert!(s.notice.is_some());
    }

    #[test]
    fn a_forbidden_escape_burns_the_turn() {
        let mut s = session();
        s.battle.escape_forbidden = true;
        run_until_turn(&mut s);
        s.input(Input::Down);
        s.input(Input::Down);
        s.input(Input::Down); // 逃跑
        s.input(Input::Confirm);
        assert!(s.turn.is_none());
        assert!(matches!(s.pending, Some(Pending::Escape { slot: 0 })));
        // 逃跑一样要走一段出手（`LAB_00515200` 的 0x51527a）。
        for _ in 0..400 {
            if s.log.back().map(|l| l.contains("逃跑")).unwrap_or(false) {
                break;
            }
            s.tick(0.05);
        }
        assert_eq!(s.finished(), None);
        assert_eq!(s.log.back().unwrap(), "景天 逃跑失败");
    }

    #[test]
    fn fast_forward_lets_the_ai_take_the_players_turn() {
        let mut s = session();
        run_until_turn(&mut s);
        s.auto_play();
        assert!(s.turn.is_none());
        assert!(!s.log.is_empty());
    }
}
