//! PAL3-specific [`agent_server`] adapter.
//!
//! The shared, game-agnostic plumbing (queue, synthetic input,
//! pause/step cells, trace ring, rendering engine) lives in
//! [`crate::agent_common::AgentBridge`]. This module:
//!
//! * Builds [`StateSnapshot`]s from PAL3 state (`AdventureDirector`,
//!   [`SceVm`](crate::scripting::sce::vm::SceVm),
//!   [`GlobalState`](crate::openpal3::states::global_state::GlobalState),
//!   [`PersistentState`](crate::openpal3::states::persistent_state::PersistentState)).
//! * Dispatches [`AgentCommand`]s to the appropriate PAL3 surface
//!   (synthetic input → bridge, teleport → director, save/load → service).
//!
//! Unsupported endpoints (no clean PAL3 mapping today) return
//! `AgentResponse::Error { kind: NotImplemented }` so external
//! drivers can probe and fall back. See `docs/agent_interface.md`
//! for the per-endpoint matrix.
//!
//! The dispatcher is invoked from `Pal3Service::pump_agent`; it never
//! crosses the HTTP↔game thread boundary directly (everything goes
//! through the shared [`AgentCommandQueue`]).

use std::rc::Rc;

use agent_server::protocol::{
    AgentCommand, AgentError, AgentResponse, AxisInputParams, CameraPoseParams, DialogSnapshot,
    KeyAction, KeyInputParams, ScreenshotResponse, ScriptEvalParams, ScriptEvalResponse,
    ScriptGlobalsParams, ScriptGlobalsResponse, SlotParams, StateSnapshot, StatusMenuParams,
    StepTimeParams, TeleportParams,
};
use crosscom::ComRc;
use radiance::comdef::ISceneManager;
use radiance::input::{Axis, Key};
use radiance::math::Vec3;

use crate::agent_common::AgentBridge;
use crate::openpal3::directors::AdventureDirector;

/// Default size of the dense window returned by `/v1/script/globals`
/// when the caller omits `limit`. PAL3 globals are written sparsely
/// by the SCE VM, so the dense window is informative even past the
/// last populated slot.
const DEFAULT_GLOBALS_WINDOW: usize = 256;

/// Maximum globals slot we'll probe when reporting the array length.
/// PAL3's SCE scripts use signed 16-bit var indices; bounding the
/// probe at 1024 covers every observed global and keeps the snapshot
/// path O(N) with a fixed cap.
const GLOBALS_PROBE_MAX: i16 = 1024;

/// Stitched together once per command by `Pal3Service::pump_agent`.
///
/// Borrows are short-lived (one command's worth) so we don't hold
/// engine borrows across the dispatcher; the service is responsible
/// for cloning out the `SceneManager` handle ahead of time.
pub struct Pal3DispatchCtx<'a> {
    pub bridge: &'a Rc<AgentBridge>,
    /// Active adventure director, if one is installed. `None` while
    /// the start menu / title is up.
    pub director: Option<&'a AdventureDirector>,
    /// Live scene manager (cloned once per pump_agent).
    pub scene_manager: ComRc<ISceneManager>,
}

/// Dispatch a single [`AgentCommand`] against the supplied PAL3
/// context. The reply is wired straight back to the HTTP client by
/// the surrounding `pump_agent` loop.
pub fn dispatch_pal3_command(ctx: &Pal3DispatchCtx, command: AgentCommand) -> AgentResponse {
    use AgentCommand as C;

    match command {
        // --- generic bridge / observability -------------------------------
        C::GetState => AgentResponse::State(build_snapshot(ctx)),
        C::KeyInput(p) => handle_key_input(ctx.bridge, p),
        C::AxisInput(p) => handle_axis_input(ctx.bridge, p),
        C::PauseTime => {
            ctx.bridge.paused.set(true);
            AgentResponse::Ok
        }
        C::ResumeTime => {
            ctx.bridge.paused.set(false);
            ctx.bridge.requested_steps.set(0);
            AgentResponse::Ok
        }
        C::StepTime(p) => handle_step(ctx.bridge, p),
        C::FastForward(p) => {
            ctx.bridge.fast_forward.set(p.on);
            AgentResponse::Ok
        }
        C::Screenshot => dispatch_screenshot(ctx.bridge),
        C::LogTail(_) => AgentResponse::err(AgentError::internal(
            "log_tail must not be queued; served by transport",
        )),
        C::GetPerfMetrics => handle_get_perf_metrics(),

        // --- PAL3-specific gameplay surface --------------------------------
        C::TeleportPlayer(p) => handle_teleport(ctx, p),
        C::AdvanceDialog => {
            // Synthesise the same key the player taps to advance text
            // (Space is the PAL3 default; mirrors PAL4's behaviour).
            ctx.bridge.input_bridge.borrow().tap(Key::Space);
            AgentResponse::Ok
        }
        C::SaveSlot(p) => handle_save_slot(ctx, p),
        C::GetScriptGlobals(p) => handle_get_globals(ctx, p),
        C::SetStatusMenu(p) => handle_set_status_menu(ctx, p),

        // --- mode control: routed through the dispatcher in service.rs ----
        // `LoadSlot` (`/v1/load`) is unified with `EnterLoadGame`: PAL3 has
        // no in-place restore, so `Pal3Service::dispatch_mode_control`
        // rebuilds a fresh `AdventureDirector` from the slot for both. All
        // four are intercepted by `is_mode_control_command` before reaching
        // this per-command dispatcher; this arm is a defensive fallback so a
        // future refactor doesn't silently swallow them.
        C::EnterNewGame | C::EnterLoadGame(_) | C::LoadSlot(_) | C::ExitGame => {
            AgentResponse::err(AgentError::internal(
                "mode-control command leaked into the PAL3 per-command dispatcher",
            ))
        }

        // --- not yet implemented for PAL3 ---------------------------------
        C::ChooseDialog(_) => AgentResponse::err(AgentError::not_implemented(
            "PAL3 dialog choice buffering not yet implemented; the SCE dialog system reads the \
             current selection via SceProcContext::set_dlgsel directly",
        )),
        C::ChooseWorldMap(_) => AgentResponse::err(AgentError::not_implemented(
            "PAL3 has no world-map prompt analog to PAL4's giShowWorldMap",
        )),
        C::GetSceneTriggers => AgentResponse::err(AgentError::not_implemented(
            "PAL3 scene triggers are SCE proc entry points; enumeration deferred",
        )),
        C::GetSceneObjects => AgentResponse::err(AgentError::not_implemented(
            "PAL3 scene object enumeration deferred (no GOB research_function analog)",
        )),
        C::FireSceneTrigger(_) => AgentResponse::err(AgentError::not_implemented(
            "PAL3 fire_trigger deferred; will call SceVm::call_proc by name once mapping lands",
        )),
        C::InteractObject(_) => AgentResponse::err(AgentError::not_implemented(
            "PAL3 has no GOB Examine handler analog",
        )),
        C::ScriptEval(p) => handle_script_eval(ctx, p),
        C::SetCamera(p) => handle_set_camera(ctx, p),
        C::TraceStart(_) | C::TraceStop | C::TraceDrain(_) => {
            AgentResponse::err(AgentError::not_implemented(
                "PAL3 SceVm has no trace adapter yet; /v1/script/trace/* unavailable",
            ))
        }

        // AgentCommand is `#[non_exhaustive]` — surface any future
        // variants as `not_implemented` rather than panicking, so a
        // workspace upgrade to a newer agent_server doesn't bring
        // PAL3 down.
        other => AgentResponse::err(AgentError::not_implemented(format!(
            "PAL3 agent dispatcher does not yet implement {other:?}",
        ))),
    }
}

/// Build a [`StateSnapshot`] from whatever PAL3 state is reachable.
/// When no `AdventureDirector` is active (start menu / title) the
/// snapshot carries just the frame/dt/fps from the bridge.
pub fn build_snapshot(ctx: &Pal3DispatchCtx) -> StateSnapshot {
    let mut snap = StateSnapshot {
        frame: ctx.bridge.frame.get(),
        paused: ctx.bridge.paused.get(),
        fast_forward: ctx.bridge.fast_forward.get(),
        fps: ctx.bridge.fps_display.get(),
        dt: ctx.bridge.dt_display.get(),
        ..Default::default()
    };

    let Some(director) = ctx.director else {
        return snap;
    };

    let sce_vm = director.sce_vm();
    let global = sce_vm.global_state();
    let persistent = global.persistent_state();

    snap.scene = persistent.scene_name().unwrap_or_default();
    snap.block = persistent.sub_scene_name().unwrap_or_default();
    snap.leader = global.role_controlled();
    if let Some(pos) = director.controlled_role_position() {
        snap.leader_pos = [pos.x, pos.y, pos.z];
    }

    // PAL3's `adv_input_enabled` is set to `false` whenever the
    // engine is mid-cutscene, dialog, or movie. Treat its negation
    // as "script is running" for the agent contract — combined with
    // `is_running()` for the case where a proc is pending but input
    // is still nominally enabled (script kick-off frame).
    let script_running = !global.adv_input_enabled() || sce_vm.state().context().is_running();
    snap.script_running = script_running;
    snap.current_script_fn = sce_vm.state().context().current_proc_name();

    // The dialog box itself lives in a p7 renderer that cannot be read
    // back, so the dialog commands mirror what they are showing onto
    // `SceState`. Without this an automation driver cannot tell "waiting
    // for the player to advance text" from "stuck".
    snap.dialog = match sce_vm.state().dialog() {
        Some(d) => DialogSnapshot {
            open: true,
            text: d.text.clone(),
            avatar: d.avatar.clone(),
            choices: d.choices.clone(),
        },
        None => DialogSnapshot::default(),
    };
    snap
}

fn handle_key_input(bridge: &Rc<AgentBridge>, params: KeyInputParams) -> AgentResponse {
    let Some(key) = Key::from_name(&params.key) else {
        return AgentResponse::err(AgentError::bad_request(format!(
            "unknown key name: {}",
            params.key
        )));
    };
    let synthetic = bridge.input_bridge.borrow();
    match params.action {
        KeyAction::Down => synthetic.press_down(key),
        KeyAction::Up => synthetic.release(key),
        KeyAction::Tap => synthetic.tap(key),
    }
    AgentResponse::Ok
}

fn handle_axis_input(bridge: &Rc<AgentBridge>, params: AxisInputParams) -> AgentResponse {
    let Some(axis) = Axis::from_name(&params.axis) else {
        return AgentResponse::err(AgentError::bad_request(format!(
            "unknown axis name: {}",
            params.axis
        )));
    };
    bridge.input_bridge.borrow().set_axis(axis, params.value);
    AgentResponse::Ok
}

fn handle_step(bridge: &Rc<AgentBridge>, params: StepTimeParams) -> AgentResponse {
    if !bridge.paused.get() {
        return AgentResponse::err(AgentError::conflict(
            "must pause time before requesting fixed-step frames",
        ));
    }
    if params.frames == 0 {
        return AgentResponse::Ok;
    }
    bridge
        .requested_steps
        .set(bridge.requested_steps.get().saturating_add(params.frames));
    bridge.requested_dt.set(params.dt.unwrap_or(0.0).max(0.0));
    AgentResponse::Ok
}

fn dispatch_screenshot(bridge: &Rc<AgentBridge>) -> AgentResponse {
    let engine = match bridge.rendering_engine.borrow().clone() {
        Some(e) => e,
        None => return AgentResponse::Screenshot(ScreenshotResponse::default()),
    };
    match engine.borrow_mut().capture_last_frame() {
        Some(frame) => AgentResponse::Screenshot(ScreenshotResponse {
            width: frame.width,
            height: frame.height,
            encoded: true,
            rgba: frame.rgba,
        }),
        None => AgentResponse::Screenshot(ScreenshotResponse::default()),
    }
}

fn handle_get_perf_metrics() -> AgentResponse {
    use agent_server::protocol::{PerfMetric, PerfMetricsResponse};

    let entries = radiance::perf::snapshot();
    let metrics = entries
        .into_iter()
        .map(|(name, snapshot)| match snapshot {
            radiance::perf::MetricSnapshot::Timing {
                calls,
                avg_ns,
                max_ns,
            } => PerfMetric::Timing {
                name: name.to_string(),
                calls,
                avg_ns,
                max_ns,
            },
            radiance::perf::MetricSnapshot::Counter { frame, total } => PerfMetric::Counter {
                name: name.to_string(),
                frame,
                total,
            },
            radiance::perf::MetricSnapshot::Gauge { last, max } => PerfMetric::Gauge {
                name: name.to_string(),
                last,
                max,
            },
        })
        .collect();
    AgentResponse::PerfMetrics(PerfMetricsResponse {
        enabled: radiance::perf::enabled(),
        metrics,
    })
}

fn handle_teleport(ctx: &Pal3DispatchCtx, params: TeleportParams) -> AgentResponse {
    let Some(director) = ctx.director else {
        return AgentResponse::err(AgentError::conflict(
            "no active adventure director — start a New Game before teleporting",
        ));
    };
    // PAL3 only models a single controlled role at any one time
    // (`GlobalState::role_controlled`); the `player` slot is
    // accepted but ignored beyond a basic bounds check so callers
    // get a clear error rather than silent acceptance of garbage.
    if params.player < 0 || params.player > 5 {
        return AgentResponse::err(AgentError::bad_request(format!(
            "PAL3 player slot must be in 0..=5, got {}",
            params.player
        )));
    }
    let pos = Vec3::new(params.pos[0], params.pos[1], params.pos[2]);
    if director.teleport_controlled_role(pos) {
        AgentResponse::Ok
    } else {
        AgentResponse::err(AgentError::conflict(
            "no scene loaded or controlled role cannot be resolved",
        ))
    }
}

/// PAL3 has no AngelScript VM, so `/v1/script/eval` carries the two
/// script pokes the debug overlay offers instead:
///
/// * `{"function": "call_proc", "args": [<proc id>]}` runs an SCE proc,
///   the way the overlay's "Execute" button does;
/// * `{"function": "load_scene", "args": [<scene>, <sub scene>]}` jumps
///   to another scene, which also swaps the running `.sce`;
/// * `{"function": "set_global", "args": [<var>, <value>]}` writes a
///   script variable — `-32768` is the main-story flag;
/// * `{"function": "set_role_alpha", "args": [<role id>, <alpha>]}` writes
///   an actor's tint alpha, the value the 135/136 fades ramp. Reaching
///   those two through a proc means playing a whole cutscene first, and
///   every proc that fades also walks and talks its way there;
/// * `{"function": "set_water_magic", "args": [0|1]}` flips 152's水特效
///   for the same reason — the only two procs that use it sit at the end
///   of long 熔岩地狱 cutscenes;
/// * `{"function": "play_effect", "args": [<group>, x, y, z]}` plays a
///   flexeff group at a point, so its timeline can be screenshotted from
///   a known start.
///
/// Both exist so an automation driver can reach a scripted beat without
/// playing up to it; commands whose only effect is on screen cannot be
/// checked any other way.
fn handle_script_eval(ctx: &Pal3DispatchCtx, params: ScriptEvalParams) -> AgentResponse {
    let Some(director) = ctx.director else {
        return AgentResponse::err(AgentError::conflict(
            "no active adventure director — start a New Game first",
        ));
    };

    let int_arg = |i: usize| -> Option<i64> { params.args.get(i).and_then(|v| v.as_i64()) };

    match params.function.as_str() {
        "call_proc" => {
            let Some(proc_id) = int_arg(0).filter(|v| *v >= 0) else {
                return AgentResponse::err(AgentError::bad_request(
                    "call_proc needs args: [proc_id]",
                ));
            };
            director.call_sce_proc(proc_id as u32);
            AgentResponse::Script(ScriptEvalResponse {
                function: params.function,
                result: None,
            })
        }
        "set_global" => {
            let (Some(var), Some(value)) = (int_arg(0), int_arg(1)) else {
                return AgentResponse::err(AgentError::bad_request(
                    "set_global needs args: [var, value]",
                ));
            };
            if !(i16::MIN as i64..=i16::MAX as i64).contains(&var) {
                return AgentResponse::err(AgentError::bad_request("var must fit in i16"));
            }
            director.set_script_global(var as i16, value as i32);
            AgentResponse::Script(ScriptEvalResponse {
                function: params.function,
                result: None,
            })
        }
        "set_water_magic" => {
            let on = int_arg(0).unwrap_or(0) != 0;
            director.set_water_magic(on);
            AgentResponse::Script(ScriptEvalResponse {
                function: params.function,
                result: None,
            })
        }
        "play_effect" => {
            let f = |i: usize| params.args.get(i).and_then(|v| v.as_f64()).unwrap_or(0.) as f32;
            let Some(group) = int_arg(0) else {
                return AgentResponse::err(AgentError::bad_request(
                    "play_effect needs args: [group, x, y, z]",
                ));
            };
            director.play_effect(group as i32, Vec3::new(f(1), f(2), f(3)));
            AgentResponse::Script(ScriptEvalResponse {
                function: params.function,
                result: None,
            })
        }
        "set_role_alpha" => {
            let alpha = params.args.get(1).and_then(|v| v.as_f64());
            let (Some(role_id), Some(alpha)) = (int_arg(0), alpha) else {
                return AgentResponse::err(AgentError::bad_request(
                    "set_role_alpha needs args: [role_id, alpha]",
                ));
            };
            let Some(n) = director.set_role_alpha(role_id as i32, alpha as f32) else {
                return AgentResponse::err(AgentError::bad_request(format!(
                    "no such role: {role_id}"
                )));
            };
            AgentResponse::Script(ScriptEvalResponse {
                function: params.function,
                result: Some(serde_json::json!({ "render_objects": n })),
            })
        }
        "object_transform" => {
            // [物件 id] —— 物件和子节点的局部矩阵，隔一会儿读两次看它动没动。
            let Some(id) = int_arg(0) else {
                return AgentResponse::err(AgentError::bad_request(
                    "object_transform needs args: [object_id]",
                ));
            };
            let Some(m) = director.object_transforms(id as i32) else {
                return AgentResponse::err(AgentError::bad_request(format!(
                    "no such object: {id}"
                )));
            };
            AgentResponse::Script(ScriptEvalResponse {
                function: params.function,
                result: Some(serde_json::json!({ "matrices": m })),
            })
        }
        "dlg_sel" => {
            // [选项, ...] —— 直接弹一个 65 对话选择框，看它的版面和操作。
            let items: Vec<String> = params
                .args
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            director.start_debug_dlg_sel(items);
            AgentResponse::Script(ScriptEvalResponse {
                function: params.function,
                result: None,
            })
        }
        "add_money" => {
            // [n] —— 加钱（负数扣钱），测买卖用。
            director.add_money(int_arg(0).unwrap_or(0) as i32);
            AgentResponse::Script(ScriptEvalResponse {
                function: params.function,
                result: None,
            })
        }
        "bigmap" => {
            // [] —— 直接开大地图（102 13）。
            director.start_debug_bigmap();
            AgentResponse::Script(ScriptEvalResponse {
                function: params.function,
                result: None,
            })
        }
        "bigmap_set" => {
            // [s0, s1, ...] —— 按下标给目的地设状态（同 125：0 藏、1 看得见去不了、2 能去）。
            let states: Vec<i32> = params
                .args
                .iter()
                .filter_map(|v| v.as_i64())
                .map(|v| v as i32)
                .collect();
            director.set_bigmap_elements(&states);
            AgentResponse::Script(ScriptEvalResponse {
                function: params.function,
                result: None,
            })
        }
        "note" => {
            // ["文字"] —— 弹一个 68 提示框。
            let text = params
                .args
                .first()
                .and_then(|v| v.as_str())
                .unwrap_or("雪岭镇")
                .to_string();
            director.show_note(text);
            AgentResponse::Script(ScriptEvalResponse {
                function: params.function,
                result: None,
            })
        }
        "caption" => {
            // ["1A", 行数] —— 86 全屏字幕。
            let name = params
                .args
                .first()
                .and_then(|v| v.as_str())
                .unwrap_or("1A")
                .to_string();
            let lines = int_arg(1).unwrap_or(4) as i32;
            director.start_debug_command(Box::new(
                crate::scripting::sce::commands::SceCommandCaption::new(name, lines),
            ));
            AgentResponse::Script(ScriptEvalResponse {
                function: params.function,
                result: None,
            })
        }
        "xjpic" => {
            // [] —— 156 雪见图片。
            director.start_debug_command(Box::new(
                crate::scripting::sce::commands::SceCommandShowXueJianPicture::new(),
            ));
            AgentResponse::Script(ScriptEvalResponse {
                function: params.function,
                result: None,
            })
        }
        "add_item" => {
            // [物品 id, 个数] —— 往背包里放东西，测冶炼、买卖用。
            let id = int_arg(0).unwrap_or(0).max(0) as u32;
            let count = int_arg(1).unwrap_or(1).max(0) as u32;
            director.add_item(id, count);
            AgentResponse::Script(ScriptEvalResponse {
                function: params.function,
                result: None,
            })
        }
        "team_add" => {
            // [角色号…] —— 往队里加人（0 景天 … 4 长卿），测买装备页的对比栏用。
            for i in 0..params.args.len() {
                if let Some(id) = int_arg(i) {
                    director.add_to_team(id.max(0) as u32);
                }
            }
            AgentResponse::Script(ScriptEvalResponse {
                function: params.function,
                result: None,
            })
        }
        "combat_combo" => {
            // [合击技 id] —— 战斗里轮到我方时直接放这条合击技（不看条件），看演出用。
            if let Some(id) = int_arg(0) {
                director.request_combat_combo(id.max(0) as u32);
            }
            AgentResponse::Script(ScriptEvalResponse {
                function: params.function,
                result: None,
            })
        }
        "lineup_set" => {
            // [角色号, 站位, 角色号, 站位…] —— 排阵法站位（0..4），测五灵盘用。
            let mut i = 0;
            while i + 1 < params.args.len() {
                if let (Some(id), Some(pos)) = (int_arg(i), int_arg(i + 1)) {
                    director.set_lineup_position(id.max(0) as u32, pos.max(0) as u8);
                }
                i += 2;
            }
            AgentResponse::Script(ScriptEvalResponse {
                function: params.function,
                result: None,
            })
        }
        "lineup_click" => {
            // [x, y] —— 阵法页的五灵盘下一帧当作在 800×600 画布这点点了一下，测选人 / 换位用。
            let f = |i: usize| params.args.get(i).and_then(|v| v.as_f64());
            let (Some(x), Some(y)) = (f(0), f(1)) else {
                return AgentResponse::err(AgentError::bad_request(
                    "lineup_click needs args: [x, y]",
                ));
            };
            crate::openpal3::lineup_disc::queue_debug_click(x as f32, y as f32);
            AgentResponse::Script(ScriptEvalResponse {
                function: params.function,
                result: None,
            })
        }
        "smelting" => {
            // [过程号] —— 直接开冶炼（146）。
            let proc_id = int_arg(0).unwrap_or(0) as i32;
            director.start_debug_command(Box::new(
                crate::scripting::sce::commands::SceCommandSmeltingMenu::new(proc_id),
            ));
            AgentResponse::Script(ScriptEvalResponse {
                function: params.function,
                result: None,
            })
        }
        "game_over" => {
            // [] —— 只弹战败画面，按一下回标题。
            director.start_debug_command(Box::new(
                crate::scripting::sce::commands::SceCommandGameOverScreen::default(),
            ));
            AgentResponse::Script(ScriptEvalResponse {
                function: params.function,
                result: None,
            })
        }
        "hockshop" => {
            // [过程号] —— 直接开当铺（103）。
            let proc_id = int_arg(0).unwrap_or(0) as i32;
            director.start_debug_command(Box::new(
                crate::scripting::sce::commands::SceCommandMiniGamePawnshop::new(proc_id),
            ));
            AgentResponse::Script(ScriptEvalResponse {
                function: params.function,
                result: None,
            })
        }
        "dealer" => {
            // ["DealScript\\WeaponShop\\q01weapon.txt"] —— 直接开一家店（SCE 100）。
            let shop = params
                .args
                .first()
                .and_then(|v| v.as_str())
                .unwrap_or("DealScript\\WeaponShop\\q01weapon.txt")
                .to_string();
            director.start_debug_dealer(shop);
            AgentResponse::Script(ScriptEvalResponse {
                function: params.function,
                result: None,
            })
        }
        "start_combat" => {
            // [id0..id5, boss?] —— 六个战斗 actor id 加一个可选的 boss 标志。
            let mut ids = [0i32; 6];
            for (i, slot) in ids.iter_mut().enumerate() {
                *slot = int_arg(i).unwrap_or(0) as i32;
            }
            let boss = int_arg(6).unwrap_or(1) != 0;
            director.start_debug_combat(ids, boss);
            AgentResponse::Script(ScriptEvalResponse {
                function: params.function,
                result: None,
            })
        }
        "load_scene" => {
            let name = params.args.first().and_then(|v| v.as_str());
            let sub = params.args.get(1).and_then(|v| v.as_str());
            let (Some(name), Some(sub)) = (name, sub) else {
                return AgentResponse::err(AgentError::bad_request(
                    "load_scene needs args: [scene, sub_scene]",
                ));
            };
            if !director.load_scene(name, sub) {
                return AgentResponse::err(AgentError::bad_request(format!(
                    "no such scene: {name}/{sub}"
                )));
            }
            AgentResponse::Script(ScriptEvalResponse {
                function: params.function,
                result: None,
            })
        }
        other => AgentResponse::err(AgentError::bad_request(format!(
            "PAL3 script_eval supports call_proc, set_global and load_scene, got {other:?}"
        ))),
    }
}

/// `/v1/camera/pose` — put the camera where the caller asks and leave it
/// there. Needed to check anything whose only evidence is on screen: a
/// scripted beat usually points the camera at what *it* cares about, not
/// at the actor whose bubble or formation is being verified.
fn handle_set_camera(ctx: &Pal3DispatchCtx, params: CameraPoseParams) -> AgentResponse {
    let Some(director) = ctx.director else {
        return AgentResponse::err(AgentError::conflict(
            "no active adventure director — start a New Game first",
        ));
    };
    let eye = Vec3::new(params.eye[0], params.eye[1], params.eye[2]);
    let target = Vec3::new(params.target[0], params.target[1], params.target[2]);
    if director.set_camera_pose(eye, target) {
        AgentResponse::Ok
    } else {
        AgentResponse::err(AgentError::conflict("no scene loaded"))
    }
}

fn handle_set_status_menu(ctx: &Pal3DispatchCtx, params: StatusMenuParams) -> AgentResponse {
    let Some(director) = ctx.director else {
        return AgentResponse::err(AgentError::conflict(
            "no active adventure director — start a New Game or load a slot before opening the \
             status menu",
        ));
    };
    // The renderer only paints (and so honors menu state) while the
    // player has control — i.e. no SCE proc is running. Toggling the
    // flag is always safe; it takes visible effect on the next idle
    // frame.
    let renderer = director.sce_vm().state().status_renderer();
    match params.page.as_deref() {
        Some(page) => {
            let index = match page {
                "state" => 0,
                "prop" => 1,
                "equip" => 2,
                "magic" => 3,
                "array" | "lineup" => 4,
                "sword" => 5,
                "book" | "justicebook" => 6,
                "system" => 7,
                other => {
                    return AgentResponse::err(AgentError::bad_request(format!(
                        "unknown page `{other}`"
                    )));
                }
            };
            renderer.set_menu_page(if params.open { index } else { -1 });
        }
        None => renderer.set_menu_open(params.open),
    }
    AgentResponse::Ok
}

fn handle_save_slot(ctx: &Pal3DispatchCtx, params: SlotParams) -> AgentResponse {
    let Some(director) = ctx.director else {
        return AgentResponse::err(AgentError::conflict(
            "no active adventure director — cannot save without an in-progress playthrough",
        ));
    };
    if params.slot < 1 || params.slot > 4 {
        return AgentResponse::err(AgentError::bad_request(format!(
            "PAL3 save slot must be in 1..=4, got {}",
            params.slot
        )));
    }
    director
        .sce_vm()
        .global_state()
        .persistent_state()
        .save(params.slot);
    AgentResponse::Ok
}

fn handle_get_globals(ctx: &Pal3DispatchCtx, params: ScriptGlobalsParams) -> AgentResponse {
    let Some(director) = ctx.director else {
        return AgentResponse::ScriptGlobals(ScriptGlobalsResponse::default());
    };
    let sce_vm = director.sce_vm();
    let persistent = sce_vm.global_state().persistent_state();

    // PAL3 globals are sparse: walk a bounded probe to determine the
    // "effective length" (one past the highest written slot) and
    // then expose a dense window over [start, start+limit).
    let mut len: usize = 0;
    for i in 0..=GLOBALS_PROBE_MAX {
        if persistent.get_global(i).is_some() {
            len = (i as usize) + 1;
        }
    }
    let start = params.start;
    let limit = params.limit.unwrap_or(DEFAULT_GLOBALS_WINDOW);
    let end = start.saturating_add(limit).min(len.max(start));
    let mut globals = Vec::with_capacity(end.saturating_sub(start));
    for i in start..end {
        let v = persistent.get_global(i as i16).unwrap_or(0);
        // The wire format is `u32`, matching PAL4's bitcast convention.
        globals.push(v as u32);
    }
    AgentResponse::ScriptGlobals(ScriptGlobalsResponse {
        len,
        start,
        globals,
        ..Default::default()
    })
}
