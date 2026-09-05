//! The script side of a battle: opcodes 73 `Combat`, 80 `CombatBoss`,
//! and the three one-shot flags 82 `CombatMaxRound`, 83 `CombatMustFail`,
//! 131 `CombatNotGameOver` that precede them. 132 `GetCombat` reads what
//! these leave behind (see `get_combat.rs`).
//!
//! How the original sequences it (`0x437280` / `0x4373d0`): fill the
//! descriptor, start the combat system (`FUN_00407320`), then park the
//! script on a type-6 waiter until the combat system reports back through
//! `FUN_004055d0` case 3, which sets `DAT_005833d0 = (result == 2)`,
//! throws the game over if the party lost while `DAT_005833cc` was set,
//! and resets the three flags. This command is that whole span: it blocks
//! the script (returns `false`) while the [`CombatSession`] runs, draws a
//! placeholder overlay for it, and on exit writes the party's state, the
//! spoils and the `GetCombat` value back.
//!
//! The overlay is text and HP bars — the 3D battle stage is not ported.
//! Everything the overlay shows comes from the rules layer, so replacing
//! it later is a rendering change only.

use crate::openpal3::combat::encounter::{CrtRand, Encounter, NO_ROUND_CAP, PendingFlags};
use crate::openpal3::combat::session::{
    Action, CombatSession, Input, Menu, MenuEntry, PlayerTurn, Profile, Reward, SkillChoice,
};
use crate::openpal3::combat::state::{Battle, Combatant, Outcome, PARTY_SLOTS, SLOTS, Side};
use crate::openpal3::directors::SceneManagerExtensions;
use crate::openpal3::scene::RoleController;
use crate::openpal3::states::game_db::{GameDb, combat_id_of_party_member};
use crate::openpal3::states::{attributes, skills};
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::{Condition, Ui};
use radiance::comdef::{IEntityExt, ISceneManager};
use radiance::input::Key;

/// `SceState::ext` key for the [`PendingFlags`] the flag opcodes set.
pub const EXT_PENDING: &str = "pal3.combat.pending";
/// `SceState::ext` key for the last battle's `GetCombat` value
/// (`DAT_005833d0`), a `bool`.
pub const EXT_RESULT: &str = "pal3.combat.result";

/// `FUN_00407320`'s fallback when the party stands outside the nav map.
const FLOOR_TYPE_OUT_OF_RANGE: i32 = 2;

fn pending_flags_mut(state: &mut SceState) -> &mut PendingFlags {
    state
        .ext_mut()
        .entry(EXT_PENDING.to_string())
        .or_insert_with(|| Box::new(PendingFlags::default()))
        .downcast_mut::<PendingFlags>()
        .expect("pal3.combat.pending holds PendingFlags")
}

/// Take the flags for the battle about to start and leave the defaults
/// behind — the original resets them only *after* the battle, but nothing
/// reads them in between, so taking them now is equivalent.
fn take_pending_flags(state: &mut SceState) -> PendingFlags {
    state
        .ext_mut()
        .remove(EXT_PENDING)
        .and_then(|b| b.downcast::<PendingFlags>().ok())
        .map(|b| *b)
        .unwrap_or_default()
}

/// What the last battle left for `GetCombat`: `None` if there was none.
pub fn last_result(state: &mut SceState) -> Option<bool> {
    state
        .ext_mut()
        .get(EXT_RESULT)
        .and_then(|b| b.downcast_ref::<bool>())
        .copied()
}

/// Opcode 82 — `DAT_00c1171c = n` (0x437f20 area).
#[derive(Debug, Clone)]
pub struct SceCommandCombatMaxRound {
    max_round: i32,
}

impl SceCommandCombatMaxRound {
    pub fn new(max_round: i32) -> Self {
        Self { max_round }
    }
}

impl SceCommand for SceCommandCombatMaxRound {
    fn update(&mut self, _: ComRc<ISceneManager>, _: &Ui, state: &mut SceState, _: f32) -> bool {
        pending_flags_mut(state).max_round = self.max_round;
        true
    }
}

/// Opcode 83 — `DAT_00c11748 = 1`.
#[derive(Debug, Clone)]
pub struct SceCommandCombatMustFail;

impl SceCommandCombatMustFail {
    pub fn new() -> Self {
        Self
    }
}

impl SceCommand for SceCommandCombatMustFail {
    fn update(&mut self, _: ComRc<ISceneManager>, _: &Ui, state: &mut SceState, _: f32) -> bool {
        pending_flags_mut(state).must_fail = true;
        true
    }
}

/// Opcode 131 — `DAT_005833cc = 0` (0x438440), the only writer of 0.
#[derive(Debug, Clone)]
pub struct SceCommandCombatNotGameOver;

impl SceCommandCombatNotGameOver {
    pub fn new() -> Self {
        Self
    }
}

impl SceCommand for SceCommandCombatNotGameOver {
    fn update(&mut self, _: ComRc<ISceneManager>, _: &Ui, state: &mut SceState, _: f32) -> bool {
        pending_flags_mut(state).game_over_on_loss = false;
        true
    }
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    /// Opcode 73.
    Normal { count: i32, ids: [u32; 3] },
    /// Opcode 80.
    Boss { ids: [u32; 6] },
}

/// Opcodes 73 and 80.
#[derive(Clone)]
pub struct SceCommandCombat {
    kind: Kind,
    session: Option<CombatSession>,
    /// Party actor id per battle slot 0..5.
    /// 在场的我方成员，**存的是队伍下标，而下标就是战斗位号**
    /// （见 `game_db::PLAYER_COMBAT_IDS` 的注释和 `combat_flow.md` 第 9.7
    /// 节）。所以拿它当槽位用，不要再枚举一次。
    party_actor_ids: Vec<u32>,
    /// Whether we switched the BGM to the battle theme and owe the scene
    /// its own back.
    switched_bgm: bool,
    game_over_on_loss: bool,
    /// The 游戏结束 screen is up, waiting for a confirm.
    game_over: bool,
}

impl std::fmt::Debug for SceCommandCombat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SceCommandCombat")
            .field("kind", &self.kind)
            .field("game_over", &self.game_over)
            .finish()
    }
}

impl SceCommandCombat {
    pub fn new(count: i32, m1: i32, m2: i32, m3: i32) -> Self {
        Self::with_kind(Kind::Normal {
            count,
            ids: [m1 as u32, m2 as u32, m3 as u32],
        })
    }

    pub fn new_boss(m1: i32, m2: i32, m3: i32, m4: i32, m5: i32, m6: i32) -> Self {
        Self::with_kind(Kind::Boss {
            ids: [m1, m2, m3, m4, m5, m6].map(|m| m as u32),
        })
    }

    fn with_kind(kind: Kind) -> Self {
        Self {
            kind,
            session: None,
            party_actor_ids: Vec::new(),
            switched_bgm: false,
            game_over_on_loss: true,
            game_over: false,
        }
    }

    /// The floor type under the controlled character — `FUN_00407320`
    /// reads byte `+5` of the nav tile, i.e. the high byte of what the
    /// loader stores as `distance_to_border`.
    fn floor_type(scene_manager: &ComRc<ISceneManager>, state: &SceState) -> i32 {
        let Some(role) = scene_manager.get_resolved_role(state, -1) else {
            return FLOOR_TYPE_OUT_OF_RANGE;
        };
        let Some(scn) = scene_manager.scn_scene() else {
            return FLOOR_TYPE_OUT_OF_RANGE;
        };
        let Some(controller) = RoleController::get_role_controller(role.clone()) else {
            return FLOOR_TYPE_OUT_OF_RANGE;
        };
        let layer = controller.inner::<RoleController>().nav_layer();
        let position = role.transform().borrow().position();
        let s = scn.inner::<crate::openpal3::scene::ScnScene>();
        let (nx, nz) = s.scene_coord_to_nav_coord(layer, &position);
        // `__ftol`: truncation, not rounding.
        s.nav()
            .get(layer, nx.trunc() as i32, nz.trunc() as i32)
            .map(|tile| ((tile.distance_to_border >> 8) & 0xff) as i32)
            .unwrap_or(FLOOR_TYPE_OUT_OF_RANGE)
    }

    fn skill_choice(db: &GameDb, id: u32) -> Option<SkillChoice> {
        let s = db.skill(id)?;
        Some(SkillChoice {
            name: s
                .name
                .as_str()
                .map(|n| n.to_string())
                .unwrap_or_else(|_| format!("仙术#{id}")),
            effect: skills::effect_of(s),
        })
    }

    /// Build the battle and the session from the party and the database.
    fn start(&mut self, scene_manager: ComRc<ISceneManager>, state: &mut SceState) {
        let flags = take_pending_flags(state);
        self.game_over_on_loss = flags.game_over_on_loss;

        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() ^ (d.as_secs() as u32))
            .unwrap_or(0x1234_5678);
        let encounter = match self.kind {
            Kind::Normal { count, ids } => {
                Encounter::scripted(&mut CrtRand::new(seed), count, ids, flags)
            }
            Kind::Boss { ids } => Encounter::boss(ids, flags),
        };

        let floor_type = Self::floor_type(&scene_manager, state);

        let Some(db) = state.global_state().game_db().cloned() else {
            log::error!("Combat: no game database; skipping the battle");
            return;
        };

        let mut battle = Battle::new(seed.wrapping_mul(0x9e37_79b9));
        if encounter.flags.max_round != NO_ROUND_CAP {
            battle.max_round = encounter.flags.max_round;
        }
        battle.must_fail = encounter.flags.must_fail;
        battle.escape_forbidden = encounter.escape_forbidden;

        let mut profiles: Vec<Profile> = (0..SLOTS).map(|_| Profile::default()).collect();

        {
            let mut persistent = state.global_state_mut().persistent_state_mut();

            let (scene, block) = (
                persistent.scene_name().unwrap_or_default(),
                persistent.sub_scene_name().unwrap_or_default(),
            );
            battle.scene_element = db
                .combat()
                .combat_scene(&scene, &block, floor_type)
                .and_then(|name| db.combat().scene_element(name));

            // ---- party ----
            let team: Vec<u32> = persistent
                .team()
                .iter()
                .copied()
                .take(PARTY_SLOTS)
                .collect();
            self.party_actor_ids = team.clone();
            // 战斗位是**按角色定的，不是按入队顺序**。原版
            // `FUN_004eea70` 里那张写死的表把战斗 actor id 映到位号
            // （0x65→0, 0x68→1, 0x69/0x6a→2, 0x6b→3, 0x66→4），而
            // `PLAYER_COMBAT_IDS` 的排法正好让**队伍下标 == 位号**。
            // 先前这里用的是 `team` 的枚举下标，队长换人整队站位就跟着
            // 移位了。
            for actor_id in team.iter().copied() {
                let slot = actor_id as usize;
                if slot >= PARTY_SLOTS {
                    log::warn!("Combat: 队伍里有下标 {slot}，超出五个战斗位");
                    continue;
                }
                let Some(role) = persistent.role(actor_id) else {
                    continue;
                };
                let Some(combat_id) = combat_id_of_party_member(actor_id) else {
                    continue;
                };
                let eff = attributes::effective(role, &db);

                let mut c = Combatant::new(combat_id, Side::Party);
                c.level = role.level;
                c.max_hp = eff.max_hp;
                c.hp = role.hp.min(eff.max_hp);
                c.max_sp = eff.max_sp;
                c.sp = role.sp.min(eff.max_sp);
                c.max_mp = eff.max_mp;
                c.mp = role.mp.min(eff.max_mp);
                c.attack = eff.attack;
                c.defense = eff.defense;
                c.speed = eff.speed;
                c.luck = eff.luck;
                for (i, v) in eff.elements.iter().enumerate() {
                    c.elements[i + 1] = *v > 0;
                }
                if c.hp <= 0 {
                    c.alive = false;
                }
                battle.place(slot, c);

                profiles[slot] = Profile {
                    name: db
                        .combat_actor(combat_id)
                        .and_then(|a| a.name.as_str().ok())
                        .unwrap_or_else(|| format!("#{combat_id}")),
                    skills: role
                        .skills
                        .iter()
                        .filter_map(|s| Self::skill_choice(&db, s.id))
                        .collect(),
                    reward: Reward::default(),
                };
            }

            // ---- enemies ----
            for (i, id) in encounter.monsters.iter().copied().enumerate() {
                if id == 0 {
                    continue;
                }
                let Some(row) = db.combat_actor(id) else {
                    log::warn!("Combat: monster {id} is not in the database");
                    continue;
                };
                let slot = PARTY_SLOTS + i;
                let mut c = Combatant::new(id, Side::Enemy);
                c.level = row.level;
                c.max_hp = row.attribute_values[0];
                c.hp = c.max_hp;
                c.max_sp = row.attribute_values[1];
                c.sp = c.max_sp;
                c.max_mp = row.attribute_values[2];
                c.mp = c.max_mp;
                c.attack = row.attribute_values[3];
                c.defense = row.attribute_values[4];
                c.speed = row.attribute_values[5];
                c.luck = row.attribute_values[6];
                for (i, v) in row.element_attribute_values.iter().enumerate() {
                    c.elements[i + 1] = *v > 0;
                }
                battle.place(slot, c);

                profiles[slot] = Profile {
                    name: row.name.as_str().unwrap_or_else(|_| format!("#{id}")),
                    skills: row.skill_ids[..3]
                        .iter()
                        .copied()
                        .filter(|s| *s != 0)
                        .filter_map(|s| Self::skill_choice(&db, s))
                        .collect(),
                    reward: Reward {
                        experience: row.experience,
                        money: row.money_when_killed as i32,
                        loot: (row.normal_loot != 0 && row.normal_loot_count > 0)
                            .then_some((row.normal_loot, row.normal_loot_count as i32)),
                    },
                };
                persistent.mark_monster_seen(id);
            }
        }

        if !encounter.keep_bgm {
            self.switched_bgm = state.global_state_mut().play_combat_bgm();
        }

        self.session = Some(CombatSession::new(
            battle,
            profiles,
            seed.rotate_left(13) ^ 0x5bd1_e995,
        ));
    }

    /// `FUN_004055d0` case 3: write the result and the party's state
    /// back, pay out, restore the music. Returns true when the game is
    /// over.
    fn finish(&mut self, state: &mut SceState, outcome: Outcome) -> bool {
        let Some(session) = self.session.as_ref() else {
            return false;
        };
        let won = outcome.counts_as_win();
        let db = state.global_state().game_db().cloned();

        {
            let mut persistent = state.global_state_mut().persistent_state_mut();
            for actor_id in self.party_actor_ids.iter().copied() {
                // 队伍下标就是战斗位号。
                let Some(c) = session.battle.get(actor_id as usize) else {
                    continue;
                };
                if let Some(role) = persistent.role_mut(actor_id) {
                    // 战后**没有**回血这一步。从 6328 个函数入口逐个反汇编扫
                    // 过（线性扫会失步，不可信），对战斗位 `+0x24c`（精）的
                    // 写只有三处：`FUN_004f2c70`（受伤/治疗）、`FUN_004f3e20`
                    // 的复活分支（置 0）、`FUN_004f8bb0`（敌方建位）。我方的
                    // 战斗精就是持久精——存读档直接读写它（0x530440 一带）。
                    // 所以倒下的人留在 0 精，要靠道具复活；先前这里 `.max(1)`
                    // 是标了 [推断] 的猜测，已按二进制改回。
                    //
                    // 代价：移植目前**没有用道具的通路**，所以倒下的成员会一直
                    // 躺到脚本调 FullTeamAtt 为止。见 combat_flow.md 第 9.8 节。
                    role.hp = c.hp;
                    role.sp = c.sp;
                    role.mp = c.mp;
                }
            }

            if won {
                let spoils = session.spoils();
                if let Some(db) = db.as_ref() {
                    for (slot, amount) in spoils.experience {
                        if let Some(actor_id) = self
                            .party_actor_ids
                            .iter()
                            .copied()
                            .find(|a| *a as usize == slot)
                        {
                            persistent.award_experience(actor_id, db, amount);
                        }
                    }
                    for (slot, skill_id) in session.casts.iter().copied() {
                        let (Some(actor_id), Some(row)) = (
                            self.party_actor_ids
                                .iter()
                                .copied()
                                .find(|a| *a as usize == slot),
                            db.skill(skill_id),
                        ) else {
                            continue;
                        };
                        persistent.use_skill(actor_id, skill_id, &row.times_before_level_up);
                    }
                }
                persistent.add_money(spoils.money);
                for (item, count) in spoils.loot {
                    if count > 0 {
                        persistent.add_item(item, count as u32);
                    }
                }
            }
        }

        state
            .ext_mut()
            .insert(EXT_RESULT.to_string(), Box::new(won));

        if self.switched_bgm {
            let g = state.global_state_mut();
            g.bgm_source().stop();
            g.play_default_bgm();
        }

        outcome == Outcome::Lose && self.game_over_on_loss
    }

    fn poll_input(state: &SceState) -> Option<Input> {
        let input = state.input();
        let pressed = |k: Key| input.get_key_state(k).pressed();
        if pressed(Key::Up) || pressed(Key::GamePadDPadUp) {
            Some(Input::Up)
        } else if pressed(Key::Down) || pressed(Key::GamePadDPadDown) {
            Some(Input::Down)
        } else if pressed(Key::Left) || pressed(Key::GamePadDPadLeft) {
            Some(Input::Left)
        } else if pressed(Key::Right) || pressed(Key::GamePadDPadRight) {
            Some(Input::Right)
        } else if pressed(Key::Space) || pressed(Key::GamePadEast) {
            Some(Input::Confirm)
        } else if pressed(Key::Escape) || pressed(Key::GamePadSouth) {
            Some(Input::Cancel)
        } else {
            None
        }
    }

    fn draw(ui: &Ui, session: &CombatSession) {
        let [window_width, _] = ui.io().display_size;
        ui.window("Combat")
            .collapsible(false)
            .title_bar(false)
            .resizable(false)
            .always_auto_resize(true)
            .position([16., 16.], Condition::Always)
            .size_constraints([window_width * 0.4, 0.], [window_width * 0.9, f32::MAX])
            .build(|| {
                let _font = radiance::imgui::game_font(radiance::imgui::GameFontSize::LARGE)
                    .map(|f| ui.push_font(f));

                ui.text(format!("第 {} 回合", session.battle.round + 1));
                if let Some((notice, _)) = session.notice.as_ref() {
                    ui.same_line();
                    ui.text_colored([1.0, 0.8, 0.3, 1.0], format!("  {notice}"));
                }

                let acting = session.turn.map(|t| t.slot);
                let cursor_target = match session.turn {
                    Some(PlayerTurn {
                        menu: Menu::Target { index, .. },
                        ..
                    }) => session.targets().get(index).copied(),
                    _ => None,
                };

                ui.separator();
                ui.text("敌方");
                for slot in PARTY_SLOTS..SLOTS {
                    if let Some(c) = session.battle.get(slot) {
                        let marker = if cursor_target == Some(slot) {
                            "▶"
                        } else {
                            " "
                        };
                        Self::hp_line(ui, marker, session.name(slot), c);
                    }
                }

                ui.separator();
                ui.text("我方");
                for slot in 0..PARTY_SLOTS {
                    if let Some(c) = session.battle.get(slot) {
                        let marker = if cursor_target == Some(slot) || acting == Some(slot) {
                            "▶"
                        } else {
                            " "
                        };
                        Self::hp_line(ui, marker, session.name(slot), c);
                        ui.same_line();
                        ui.text(format!("气{}/{} 神{}/{}", c.sp, c.max_sp, c.mp, c.max_mp));
                    }
                }

                ui.separator();
                for line in session.log.iter() {
                    ui.text(line);
                }

                if let Some(turn) = session.turn {
                    ui.separator();
                    Self::draw_menu(ui, session, turn);
                }
            });
    }

    fn hp_line(ui: &Ui, marker: &str, name: &str, c: &Combatant) {
        let fraction = if c.max_hp > 0 {
            (c.hp as f32 / c.max_hp as f32).clamp(0., 1.)
        } else {
            0.
        };
        let status = if !c.alive { " 倒下" } else { "" };
        ui.text(format!("{marker}{name:<8}"));
        ui.same_line();
        imgui::ProgressBar::new(fraction)
            .size([160., 0.])
            .overlay_text(format!("{}/{}{}", c.hp, c.max_hp, status))
            .build(ui);
    }

    fn draw_menu(ui: &Ui, session: &CombatSession, turn: PlayerTurn) {
        let who = session.name(turn.slot);
        match turn.menu {
            Menu::Main { index } => {
                ui.text(format!("{who}："));
                for (i, entry) in MenuEntry::ALL.iter().enumerate() {
                    ui.same_line();
                    ui.text(format!(
                        "{}{}",
                        if i == index { "▶" } else { " " },
                        entry.label()
                    ));
                }
            }
            Menu::Skills { index } => {
                ui.text(format!("{who} 仙术："));
                for (i, s) in session.skills(turn.slot).iter().enumerate() {
                    let affordable = session.can_afford(turn.slot, i);
                    let line = format!(
                        "{}{} (气{} 神{})",
                        if i == index { "▶" } else { " " },
                        s.name,
                        s.effect.sp_cost,
                        s.effect.mp_cost
                    );
                    if affordable {
                        ui.text(line);
                    } else {
                        ui.text_disabled(line);
                    }
                }
            }
            Menu::Target { action, .. } => {
                let what = match action {
                    Action::Attack => "攻击".to_string(),
                    Action::Cast(i) => session
                        .skills(turn.slot)
                        .get(i)
                        .map(|s| s.name.clone())
                        .unwrap_or_default(),
                };
                ui.text(format!("{who} {what} → 选择目标（上下切换，确定）"));
            }
        }
    }

    fn draw_game_over(ui: &Ui) {
        let [window_width, window_height] = ui.io().display_size;
        ui.window("GameOver")
            .collapsible(false)
            .title_bar(false)
            .resizable(false)
            .always_auto_resize(true)
            .position_pivot([0.5, 0.5])
            .position([window_width / 2., window_height / 2.], Condition::Always)
            .build(|| {
                let _font = radiance::imgui::game_font(radiance::imgui::GameFontSize::LARGE)
                    .map(|f| ui.push_font(f));
                ui.text("游戏结束");
                ui.text("按确定退出");
            });
    }
}

impl SceCommand for SceCommandCombat {
    fn initialize(&mut self, scene_manager: ComRc<ISceneManager>, state: &mut SceState) {
        self.start(scene_manager, state);
    }

    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        if self.game_over {
            Self::draw_game_over(ui);
            // The original switches the game's state object to the
            // title screen; the same request 147 Switch2Menu makes.
            if Self::poll_input(state) == Some(Input::Confirm) || state.fast_forward() {
                log::info!("Combat: 游戏结束，返回主菜单");
                state.request_title();
                return true;
            }
            return false;
        }

        let Some(session) = self.session.as_mut() else {
            // No database or nothing to fight: report a win so the script
            // moves on.
            state
                .ext_mut()
                .insert(EXT_RESULT.to_string(), Box::new(true));
            return true;
        };

        // Fast-forward drives the whole battle in one frame, the AI taking
        // the party's turns; a real frame takes one step.
        let outcome = if state.fast_forward() {
            let mut outcome = None;
            for _ in 0..100_000 {
                session.auto_play();
                outcome = session.tick(0.1);
                if outcome.is_some() {
                    break;
                }
            }
            outcome.or(Some(Outcome::Win))
        } else {
            if let Some(input) = Self::poll_input(state) {
                session.input(input);
            }
            session.tick(delta_sec)
        };

        if let Some(session) = self.session.as_ref() {
            Self::draw(ui, session);
        }

        match outcome {
            None => false,
            Some(outcome) => {
                log::info!("Combat: finished with {outcome:?}");
                if self.finish(state, outcome) {
                    self.game_over = true;
                    false
                } else {
                    true
                }
            }
        }
    }
}
