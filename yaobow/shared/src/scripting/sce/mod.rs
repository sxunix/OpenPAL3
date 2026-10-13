use std::{
    any::Any,
    cell::{Ref, RefCell},
    collections::HashMap,
    rc::Rc,
};

use crosscom::ComRc;
use imgui::Ui;
use radiance::{
    audio::AudioEngine, comdef::ISceneManager, input::InputEngine, radiance::UiManager,
};
use radiance_scripting::UiManagerImmediateExt;

use crate::openpal3::{
    asset_manager::AssetManager,
    comdef::{IPal3DialogRenderer, IPal3StatusRenderer},
    loaders::sce_loader::SceFile,
    states::global_state::GlobalState,
};

use self::vm::{SceExecutionContext, SceExecutionOptions};

pub mod commands;
pub mod vm;

/// The scripted PAL3 dialog-box renderer, threaded from `Pal3Service`
/// into the SCE command loop. The renderer is self-contained (it owns its
/// sprites, avatar and curtain state in p7); the host only forwards SCE
/// events and the per-frame draw calls. The imgui texture cache the draws
/// resolve through is engine-owned (reached via `UiManager::with_ui_host`),
/// so the renderer is passed straight through (no wrapper struct).

pub trait SceCommandDebug {
    fn debug(&self) -> String;
}

pub trait SceCommand: dyn_clone::DynClone + SceCommandDebug {
    fn initialize(&mut self, _scene_manager: ComRc<ISceneManager>, _state: &mut SceState) {}

    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool;
}

dyn_clone::clone_trait_object!(SceCommand);

impl<T: SceCommand + std::fmt::Debug> SceCommandDebug for T {
    fn debug(&self) -> String {
        format!("{:?}", self)
    }
}

pub struct SceState {
    asset_mgr: Rc<AssetManager>,
    global_state: GlobalState,
    context: SceExecutionContext,
    curtain: f32,
    /// Engine-side door-transition state: `SceCommandLoadScene` snaps the
    /// curtain to black on an unscripted scene swap and sets this; the VM
    /// then eases the curtain back to 0. Scripted fades clear it.
    auto_fade_in: bool,
    /// Agent-server fast-forward flag, refreshed once per frame from
    /// [`AgentBridge::fast_forward`](crate::agent_common::AgentBridge)
    /// by `AdventureDirector` before each `SceVm::update`. SCE commands
    /// read it via [`SceState::fast_forward`] to skip input-blocked
    /// waits (dialog / movie) and collapse timed tweens to their final
    /// state in a single frame. Defaults to `false` (real-time).
    fast_forward: bool,
    ext: HashMap<String, Box<dyn Any>>,
    input_engine: Rc<RefCell<dyn InputEngine>>,
    audio_engine: Rc<dyn AudioEngine>,

    // Scripted PAL3 dialog box: the SCE command loop forwards events to
    // the self-contained p7 `dialog_renderer` and drives its draw calls
    // via `UiManager::with_ui_host` — radiance's immediate-mode UI
    // composition primitive (the peer of the retained `UiLayerStack`;
    // both compose onto the same engine imgui frame through one shared
    // engine-owned texture cache). The dialog uses the immediate path
    // rather than a registered `IUiLayer` because the `dlg` SCE command
    // reads input and emits its draw in the *same* update tick (see
    // `commands/dlg.rs`), which a pre-update retained layer can't express
    // without a separate visibility state machine.
    ui: Rc<UiManager>,
    dialog_renderer: ComRc<IPal3DialogRenderer>,
    // Scripted top-right status indicator; drawn each frame the player
    // has control (no SCE running). Same immediate-mode draw path as the
    // dialog renderer.
    status_renderer: ComRc<crate::openpal3::comdef::IPal3StatusRenderer>,
    /// 68 Note's toast: the text and how long it still has to live. The
    /// command does not block, so the vm draws and ages it.
    note: Option<(String, f32)>,
    /// 提示框的九宫格外框 `ui\flex\note0..8.tga`，第一次显示时读。
    note_frame: Vec<Option<std::rc::Rc<radiance::rendering::Sprite>>>,
    /// 主角头上「跳」的图标：左上角在屏幕上的归一化坐标（x、y ∈ −1..1，y 朝上）；`None` 收起。
    jump_icon: Option<[f32; 2]>,
    /// 图标的帧（`ui\scene\tiao%d.tga`，第一次显示时按编号一直读到没有），和已经显示了多久。
    jump_icon_frames: Option<Vec<std::rc::Rc<radiance::rendering::Sprite>>>,
    jump_icon_clock: f32,
    /// What the dialog box is showing, so the agent snapshot can report
    /// it. The box itself lives in a p7 renderer that cannot be read
    /// back, and an automation driver has no other way to tell "waiting
    /// for the player to advance text" from "stuck".
    dialog: Option<DialogState>,
    /// 战斗的摘要，也是给 agent 快照看的：战斗命令每帧写一次，散场后留着
    /// （`outcome` 填上、`active` 落下），下一场开打时覆盖。没有它驱动脚本
    /// 只能盲按，分不出「轮到我方选行动」和「角色正在跑动出手」。
    combat: Option<CombatMirror>,
    /// 152 玩家周身的水特效开关。
    water_magic: bool,
    /// 140 开关下雪 — the flag at `0xbfda40` the frame loop tests
    /// before `gbDropGroup::Render`.
    snow: bool,
    /// 141 设置全局屏幕特效: the full-screen picture to lay over the
    /// frame, once loaded. `None` when no effect is on.
    screen_effect: Option<Rc<radiance::rendering::Sprite>>,
    /// 147 退出游戏到主菜单 asked for the title screen. `FUN_0044f610(4)`
    /// swaps the game's whole state object; here the host has to build
    /// the title director, so the request is parked until it looks.
    title_requested: bool,
    /// 143 CEft 排下的战斗特效：`(特效组号, 播放位置)`。指令本身不阻塞，
    /// 由 `AdventureDirector` 每帧取走并接进场景。
    effect_requests: Vec<(i32, radiance::math::Vec3)>,
    /// 特效 `camera` 类要战斗镜头播的 flexcam 序列（≤ 0 = 只停）。
    /// `AdventureDirector` 放进来，战斗命令每帧取走。
    combat_camera_requests: Vec<i32>,
    /// 特效 `action` 类要舞台上的人做的动作，同样由战斗命令每帧取走。
    combat_action_requests: Vec<crate::openpal3::combat::stage_action::ActionCue>,
    /// 特效 `quake` 类要的镜头震动 `[幅度, 每秒衰减, 频率]`，只留最新一个。
    combat_shake_request: Option<[f32; 3]>,
    /// 调试口要战斗里直接放的合击技 id（agent `combat_combo`）。
    combat_combo_request: Option<u32>,
    /// 特效 `bink` 类要播的 2D 动画（`movie/2DEFFECT` 下的名字，如 `t102`）。
    combat_movie_requests: Vec<String>,
}

/// The dialog box's readable state — mirrors the agent contract's
/// `DialogSnapshot`.
#[derive(Debug, Clone, Default)]
pub struct DialogState {
    pub text: String,
    /// `"left"`, `"right"`, or empty when no avatar is up.
    pub avatar: String,
    /// Choices offered by a select-dialog, empty otherwise.
    pub choices: Vec<String>,
}

/// 战斗的摘要，见 [`SceState::combat`]。只用平铺的类型，不把战斗模块的
/// 类型漏到脚本层来。
#[derive(Debug, Clone, Default)]
pub struct CombatMirror {
    /// 战斗还在进行（胜负出来之前）。
    pub active: bool,
    /// 轮到哪个战斗位选行动（菜单开着、战斗停着等输入）；`None` = 没人在等。
    pub turn_slot: Option<usize>,
    /// 菜单停在哪一层：`main` / `skills` / `specials` / `items` / `target` /
    /// `ai` / `lineup` / `protect`。
    pub menu: Option<&'static str>,
    /// 舞台在演一次出手（这期间时钟停着）。
    pub staging: bool,
    /// 有人登记了行动、还在出手段。
    pub pending_slot: Option<usize>,
    pub units: Vec<CombatUnitMirror>,
    /// 胜负：`win` / `lose` / `escaped` / `round_out`；没出来是 `None`。
    pub outcome: Option<&'static str>,
}

#[derive(Debug, Clone, Default)]
pub struct CombatUnitMirror {
    pub slot: usize,
    pub actor_id: u32,
    pub hp: i32,
    pub max_hp: i32,
    pub alive: bool,
    pub enemy: bool,
}

/// 68 / 157 提示框停多久：`FUN_00450c90` 在模式 2 时 `+0x108 += dt`，超过 4.0 秒
/// 直接隐藏，没有淡出。逆向见 `switch/re/notes/note_box.md`。
pub const NOTE_SECONDS: f32 = 4.0;

impl SceState {
    pub fn new(
        input_engine: Rc<RefCell<dyn InputEngine>>,
        audio_engine: Rc<dyn AudioEngine>,
        ui: Rc<UiManager>,
        asset_mgr: Rc<AssetManager>,
        sce: Rc<SceFile>,
        sce_name: String,
        global_state: GlobalState,
        options: Option<SceExecutionOptions>,
        dialog_renderer: ComRc<IPal3DialogRenderer>,
        status_renderer: ComRc<IPal3StatusRenderer>,
    ) -> Self {
        let ext = HashMap::<String, Box<dyn Any>>::new();

        Self {
            asset_mgr: asset_mgr.clone(),
            global_state,
            context: SceExecutionContext::new(sce, sce_name, options),
            curtain: 1.,
            auto_fade_in: false,
            fast_forward: false,
            ext,
            input_engine,
            audio_engine,
            ui,
            dialog_renderer,
            status_renderer,
            note: None,
            note_frame: Vec::new(),
            jump_icon: None,
            jump_icon_frames: None,
            jump_icon_clock: 0.,
            dialog: None,
            combat: None,
            snow: false,
            water_magic: false,
            screen_effect: None,
            title_requested: false,
            effect_requests: vec![],
            combat_camera_requests: vec![],
            combat_action_requests: vec![],
            combat_shake_request: None,
            combat_combo_request: None,
            combat_movie_requests: vec![],
        }
    }

    pub fn global_state(&self) -> &GlobalState {
        &self.global_state
    }

    pub fn global_state_mut(&mut self) -> &mut GlobalState {
        &mut self.global_state
    }

    pub fn context_mut(&mut self) -> &mut SceExecutionContext {
        &mut self.context
    }

    pub fn context(&self) -> &SceExecutionContext {
        &self.context
    }

    pub fn input(&self) -> Ref<'_, dyn InputEngine> {
        self.input_engine.borrow()
    }

    pub fn audio_engine(&self) -> &Rc<dyn AudioEngine> {
        &self.audio_engine
    }

    /// 正在跑的那个过程的运行模式（见 [`SceExecutionContext::run_mode`]）。
    pub fn run_mode(&self) -> i32 {
        self.context.run_mode()
    }

    pub fn set_run_mode(&mut self, run_mode: i32) {
        self.context.set_run_mode(run_mode);
    }

    /// Whether the agent-server fast-forward mode is active this frame.
    /// SCE commands consult this to skip input-blocked waits (dialog /
    /// movie) and to collapse timed tweens to their final state.
    pub fn fast_forward(&self) -> bool {
        self.fast_forward
    }

    /// Set by `AdventureDirector` once per frame from the agent bridge.
    pub fn set_fast_forward(&mut self, fast_forward: bool) {
        self.fast_forward = fast_forward;
    }

    pub fn auto_fade_in(&self) -> bool {
        self.auto_fade_in
    }

    pub fn set_auto_fade_in(&mut self, v: bool) {
        self.auto_fade_in = v;
    }

    pub fn curtain(&self) -> f32 {
        self.curtain
    }

    pub fn set_curtain(&mut self, curtain: f32) {
        let curtain = if curtain > 1. {
            1.
        } else if curtain < -1. {
            -1.
        } else {
            curtain
        };
        self.curtain = curtain;
    }

    pub fn ext_mut(&mut self) -> &mut HashMap<String, Box<dyn Any>> {
        &mut self.ext
    }

    /// What the dialog box is showing, or `None` when it is closed.
    pub fn dialog(&self) -> Option<&DialogState> {
        self.dialog.as_ref()
    }

    /// Called by the dialog commands each frame the box is up.
    pub fn set_dialog_text(&mut self, text: &str) {
        match self.dialog.as_mut() {
            Some(d) => {
                if d.text != text {
                    text.clone_into(&mut d.text);
                }
            }
            None => {
                self.dialog = Some(DialogState {
                    text: text.to_string(),
                    ..Default::default()
                })
            }
        }
    }

    pub fn set_dialog_choices(&mut self, choices: Vec<String>) {
        self.dialog.get_or_insert_with(Default::default).choices = choices;
    }

    pub fn close_dialog(&mut self) {
        self.dialog = None;
    }

    /// 最近一场战斗的摘要（还在打，或者刚散场）。
    pub fn combat(&self) -> Option<&CombatMirror> {
        self.combat.as_ref()
    }

    pub fn set_combat(&mut self, mirror: CombatMirror) {
        self.combat = Some(mirror);
    }

    /// 战斗指令被调试入口丢掉时，摘要不再更新，留着会让驱动以为还在打。
    pub fn clear_combat(&mut self) {
        self.combat = None;
    }

    /// 152: whether the water effect is on the player.
    pub fn water_magic(&self) -> bool {
        self.water_magic
    }

    pub fn set_water_magic(&mut self, on: bool) {
        self.water_magic = on;
    }

    /// 140: whether it is snowing.
    pub fn snow(&self) -> bool {
        self.snow
    }

    pub fn set_snow(&mut self, snow: bool) {
        self.snow = snow;
    }

    /// 141: put a full-screen picture over everything, or clear it.
    pub fn set_screen_effect(&mut self, sprite: Option<Rc<radiance::rendering::Sprite>>) {
        self.screen_effect = sprite;
    }

    /// Draw the 141 overlay. The original renders a UI quad spanning the
    /// whole viewport in NDC with uv (0,0)-(1,1) and vertex colour white
    /// (`gbDynVertBuf::RenderUIQuad` at 0x4b6e20), so it stretches rather
    /// than fits.
    pub fn render_screen_effect(&self, ui: &imgui::Ui) {
        let Some(sprite) = self.screen_effect.as_ref() else {
            return;
        };
        let size = ui.io().display_size;
        let _pad = ui.push_style_var(imgui::StyleVar::WindowPadding([0., 0.]));
        let _border = ui.push_style_var(imgui::StyleVar::WindowBorderSize(0.));
        ui.window("ScreenEffect")
            .collapsible(false)
            .title_bar(false)
            .resizable(false)
            .movable(false)
            .scroll_bar(false)
            .draw_background(false)
            .no_inputs()
            .position([0., 0.], imgui::Condition::Always)
            .size(size, imgui::Condition::Always)
            .build(|| {
                imgui::Image::new(sprite.imgui_texture_id(), size).build(ui);
            });
    }

    /// 147 asks to leave for the title screen.
    pub fn request_title(&mut self) {
        self.title_requested = true;
    }

    /// Read and clear the title request. Called by the host once a
    /// frame, from outside any director update — swapping the director
    /// from inside one would alias its own update.
    pub fn take_title_request(&mut self) -> bool {
        std::mem::take(&mut self.title_requested)
    }

    /// 143 CEft：排一个特效组，位置是 142/144 刚设的那个。
    pub fn play_effect(&mut self, group_id: i32, position: radiance::math::Vec3) {
        self.effect_requests.push((group_id, position));
    }

    pub fn take_effect_requests(&mut self) -> Vec<(i32, radiance::math::Vec3)> {
        std::mem::take(&mut self.effect_requests)
    }

    /// 特效 `camera` 类（FUN_004c39a0）：先停当前镜头序列（`FUN_00505800`），
    /// `seq > 0` 再从当前机位播 flexcam 第 `seq` 个序列（`FUN_00505770`）。
    ///
    /// 每个请求都会先停掉前一个，所以只留最新的一个——战斗外 143 播到 camera
    /// 类时也不会越积越多。
    pub fn request_combat_camera(&mut self, seq: i32) {
        self.combat_camera_requests.clear();
        self.combat_camera_requests.push(seq);
    }

    pub fn take_combat_camera_requests(&mut self) -> Vec<i32> {
        std::mem::take(&mut self.combat_camera_requests)
    }

    /// 特效 `action` 类（FUN_004c0c10）的一条动作提示。战斗外没人取，只留最近 64 条。
    pub fn request_combat_action(&mut self, cue: crate::openpal3::combat::stage_action::ActionCue) {
        if self.combat_action_requests.len() >= 64 {
            self.combat_action_requests.remove(0);
        }
        self.combat_action_requests.push(cue);
    }

    /// 特效 `quake` 类（0x4d35e0）：写战斗镜头的震动参数。
    /// 调试：让战斗在下一个我方回合直接放这条合击技（不管条件）。
    pub fn request_combat_combo(&mut self, id: u32) {
        self.combat_combo_request = Some(id);
    }

    pub fn take_combat_combo_request(&mut self) -> Option<u32> {
        self.combat_combo_request.take()
    }

    pub fn request_combat_shake(&mut self, amplitude: f32, decay: f32, frequency: f32) {
        self.combat_shake_request = Some([amplitude, decay, frequency]);
    }

    pub fn take_combat_shake_request(&mut self) -> Option<[f32; 3]> {
        self.combat_shake_request.take()
    }

    /// 特效 `bink` 类（FUN_004c24b0）：播一段 2D 动画。战斗外没人取，只留最近 8 条。
    pub fn request_combat_movie(&mut self, name: String) {
        if self.combat_movie_requests.len() >= 8 {
            self.combat_movie_requests.remove(0);
        }
        self.combat_movie_requests.push(name);
    }

    pub fn take_combat_movie_requests(&mut self) -> Vec<String> {
        std::mem::take(&mut self.combat_movie_requests)
    }

    pub fn take_combat_action_requests(
        &mut self,
    ) -> Vec<crate::openpal3::combat::stage_action::ActionCue> {
        std::mem::take(&mut self.combat_action_requests)
    }

    pub fn show_note(&mut self, text: String) {
        self.note = Some((text, NOTE_SECONDS));
    }

    /// 设主角头上「跳」的图标（`FUN_0041d9a0` → `FUN_00445e70` 开关、`FUN_00445d20` 挪位置）。
    pub fn set_jump_icon(&mut self, ndc: Option<[f32; 2]>) {
        self.jump_icon = ndc;
    }

    /// 画「跳」的图标（控件在 `FUN_00408be0` 建：40×32，`ui\scene\tiao%d.tga` 逐帧，每秒 4 帧循环——`FUN_0043a3e0`
    /// 攒够 1/4 秒换下一帧）。左上角是归一化坐标换成像素：x = (ndc.x + 1)·W/2，y = (1 − ndc.y)·H/2（0x41dceb 起）。
    pub fn render_jump_icon(&mut self, ui: &imgui::Ui, delta_sec: f32) {
        const FPS: f32 = 4.;
        let Some([nx, ny]) = self.jump_icon else {
            self.jump_icon_clock = 0.;
            return;
        };
        if self.jump_icon_frames.is_none() {
            let mut frames = vec![];
            while let Some(s) = self.asset_mgr.load_sprite(format!(
                "/basedata/basedata/ui/scene/tiao{}.tga",
                frames.len()
            )) {
                frames.push(s);
            }
            self.jump_icon_frames = Some(frames);
        }
        let Some(frames) = self.jump_icon_frames.as_ref().filter(|f| !f.is_empty()) else {
            return;
        };
        self.jump_icon_clock += delta_sec;
        let tex = frames[(self.jump_icon_clock * FPS) as usize % frames.len()].clone();
        let [sw, sh] = ui.io().display_size;
        let (x, y) = ((nx + 1.) * sw * 0.5, (1. - ny) * sh * 0.5);
        crate::scripting::sce::commands::overlay(ui, "JumpIcon", |ui, _at, scale| {
            ui.get_window_draw_list()
                .add_image(
                    tex.imgui_texture_id(),
                    [x, y],
                    [x + 40. * scale, y + 32. * scale],
                )
                .build();
        });
    }

    /// 画提示框、到点收起（`FUN_00450bc0` 排版，`FUN_00445500` 画）。
    ///
    /// 框宽 `max(0x40, (GBK 字节数 + 5) × 10)`、高 0x30，居中在
    /// `((800 − 0x112) / 2, 10, +0x112, 0x57)` 那条横带里——在屏幕上方。外框是
    /// `ui\flex\note` 九宫格，角块边长取 min(32, 半宽, 半高)；字是字号档 2、居中，
    /// 颜色是控件默认的 0xff000505（近黑）。
    pub fn render_note(&mut self, ui: &imgui::Ui, delta_sec: f32) {
        let Some((text, remaining)) = self.note.as_mut() else {
            return;
        };
        *remaining -= delta_sec;
        if *remaining <= 0. {
            self.note = None;
            return;
        }
        let text = text.clone();
        if self.note_frame.is_empty() {
            self.note_frame = (0..9)
                .map(|i| {
                    self.asset_mgr
                        .load_sprite(format!("/basedata/basedata/ui/flex/note{i}.tga"))
                })
                .collect();
        }

        let bytes = crate::scripting::sce::commands::dlg_sel::gbk_len(&text) as i32;
        // `UINote::Note`（`gameui/uinote.cpp:44`）：`max(64, strlen(str)*10 + 60)`，
        // 高 48。`strlen` 数的是 GBK 字节。
        let (w, h) = (0x40.max(bytes * 10 + 60), 0x30);
        let strip = ((800 - 0x112) / 2, 10, (800 - 0x112) / 2 + 0x112, 0x57);
        let x = strip.0 + (strip.2 - strip.0 - w) / 2;
        let y = strip.1 + (strip.3 - strip.1 - h) / 2;
        let b = (x, y, x + w, y + h);
        let c = 32.min(w / 2).min(h / 2);
        let (ix0, iy0, ix1, iy1) = (b.0 + c, b.1 + c, b.2 - c, b.3 - c);
        let pieces = [
            (b.0, b.1, ix0, iy0),
            (b.0, iy1, ix0, b.3),
            (ix1, iy1, b.2, b.3),
            (ix1, b.1, b.2, iy0),
            (b.0, iy0, ix0, iy1),
            (ix0, iy1, ix1, b.3),
            (ix1, iy0, b.2, iy1),
            (ix0, b.1, ix1, iy0),
            (ix0, iy0, ix1, iy1),
        ];
        let frame = &self.note_frame;
        crate::scripting::sce::commands::overlay(ui, "Note", |ui, at, _scale| {
            let list = ui.get_window_draw_list();
            // 每一块都是整张贴图缩放进去（`FUN_00445760` 的 uv 固定 0..1），
            // 不是按原生尺寸裁——框只有 48 高时角块缩成 24，顶边才不会丢。
            for (k, r) in pieces.iter().enumerate() {
                let Some(tex) = frame.get(k).and_then(|t| t.as_deref()) else {
                    continue;
                };
                if r.0 < r.2 && r.1 < r.3 {
                    list.add_image(tex.imgui_texture_id(), at(r.0, r.1), at(r.2, r.3))
                        .build();
                }
            }
            let _font = radiance::imgui::game_font(radiance::imgui::GameFontSize::LARGE)
                .map(|f| ui.push_font(f));
            let [tw, th] = ui.calc_text_size(&text);
            let [x0, y0] = at(b.0, b.1);
            let [x1, y1] = at(b.2, b.3);
            // 字色在控件类的构造里：`UIStaticFlex::UIStaticFlex`
            // （`ui/UIStaticFlex.cpp:16`）把从 `UIWnd` 继承来的近黑
            // （`gbColorQuad(5,5,0)`，`ui/uiwnd.cpp:22`）改成了
            // RGB(255, 255, 250)。`UINote` 用的正是这个类，自己没再设色。
            // `UIStaticFlex::Render`（同文件 `:214`）还写死 align = 1，横竖都居中。
            let ink = [1.0, 1.0, 250. / 255., 1.0];
            list.add_text(
                [x0 + (x1 - x0 - tw) * 0.5, y0 + (y1 - y0 - th) * 0.5],
                ink,
                &text,
            );
        });
    }

    pub fn asset_mgr(&self) -> &Rc<AssetManager> {
        &self.asset_mgr
    }

    pub fn get_next_cmd(&mut self) -> Option<Box<dyn SceCommand>> {
        self.context.get_next_cmd(&mut self.global_state)
    }

    pub fn call_proc(&mut self, proc_id: u32) {
        self.context.call_proc(proc_id, &mut self.global_state);
    }

    /// 跑另一个 `.sce` 里的过程——大地图选完目的地要跑
    /// `ui\\bigmap\\bigmap.sce` 的过程。
    pub fn call_proc_in(&mut self, sce: Rc<SceFile>, sce_name: &str, proc_id: u32) -> bool {
        self.context
            .call_proc_in(sce, sce_name, proc_id, &mut self.global_state)
    }

    pub fn try_call_proc_by_name(&mut self, proc_name: &str) {
        self.context
            .try_call_proc_by_name(proc_name, &mut self.global_state);
    }

    pub fn has_proc(&self, proc_id: u32) -> bool {
        self.context.has_proc(proc_id)
    }

    /// Compose the scripted dialog panel for this frame: drive the p7
    /// `render_dialog` with the markup-bearing `text` inside a mid-frame
    /// imgui scope. Mirrors the legacy `DialogBox::draw`.
    pub fn render_dialog(&mut self, text: &str, delta_sec: f32) {
        let renderer = self.dialog_renderer.clone();
        self.ui.with_ui_host(|ui_host| {
            renderer.render_dialog(ui_host.clone(), text, delta_sec);
        });
    }

    /// Paint the scene-transition curtain/fade for this frame from the
    /// current `curtain` value. Mirrors the legacy
    /// `DialogBox::fade_window` driven by `SceVm::draw_curtain`.
    pub fn render_curtain(&mut self) {
        let curtain = self.curtain;
        self.dialog_renderer.set_curtain(curtain);
        let renderer = self.dialog_renderer.clone();
        self.ui.with_ui_host(|ui_host| {
            renderer.render_curtain(ui_host.clone());
        });
    }

    /// Set the dialog avatar/portrait (PAL3 `dlgface`). The p7 renderer
    /// loads the portrait sprite; called outside any `with_ui_host`
    /// scope so the texture-cache upload doesn't re-enter a held borrow.
    pub fn set_dialog_avatar(&mut self, role: &str, face: &str, side: i32) {
        self.dialog_renderer.set_avatar(role, face, side);
        let side = if side == 0 { "left" } else { "right" };
        self.dialog.get_or_insert_with(Default::default).avatar = side.to_string();
    }

    /// Clear the dialog avatar/portrait (end of a `dlg` beat).
    pub fn clear_dialog_avatar(&mut self) {
        self.dialog_renderer.clear_avatar();
        if let Some(d) = self.dialog.as_mut() {
            d.avatar.clear();
        }
    }

    /// Compose the top-right status indicator for this frame. Called only
    /// when the player has control (no SCE proc running).
    pub fn render_status(&mut self, delta_sec: f32) {
        let renderer = self.status_renderer.clone();
        self.ui.with_ui_host(|ui_host| {
            renderer.render_status(ui_host.clone(), delta_sec);
        });
    }

    /// The scripted status indicator/menu renderer. Exposed so the agent
    /// server can open/close the in-game character-status (状态) menu
    /// without forging a synthetic avatar click.
    pub fn status_renderer(&self) -> ComRc<crate::openpal3::comdef::IPal3StatusRenderer> {
        self.status_renderer.clone()
    }
}
