//! 105 打苍蝇 — `LAB_004370e0`（模式 9）。
//!
//! 规则在 `openpal3::states::killflyer`；界面照源码 `subgame/KillFlyer/KF_Sys.cpp`（exe `FUN_00529870`
//! 开局、`FUN_00529e40` 每帧、`FUN_0052aa50` 收尾，见 `switch/re/notes/killflyer.md`）：
//!
//! - 背景是进来那一刻的场景截图，以 62/255 的不透明度贴在黑底上（`RenderIntercept`）；这边场景还在下面
//!   画着，盖一层 (255 − 62)/255 的黑，得到同样的暗度 [推断：原版截的是定格画面，这里背景还会动]；
//! - 场地中间先闪 2.2 秒「减血N」（AssisLang 243，`UIStaticRM` 每帧 alpha −7 / +6，6..255）；
//! - 苍蝇 32×32，普通 / 金色两张图，转向飞的方向；拍中换 `FlyerDie.tga` 往下掉；
//! - 拍子 `Kill.tga`（64×58，左上角在鼠标 −(32, 29)）跟着鼠标走，左键拍下换 `Kill_pa.tga`、放 wa040，
//!   100 毫秒后换回；场地里藏起鼠标；有苍蝇时一直放嗡嗡声 wa039；
//! - 一波结束弹 `UI\Flex\Note` 九宫格（300, 158, 132×132，0.5 秒余弦缓动从中心 32×32 展开），展开完写
//!   「打死苍蝇 N 只 / 获得金钱 M」（258 / 259），再点一下离开。
//!
//! 结算：钱加身上，花费（当前精的百分之一，至少 1）从景天的精里扣，扣到 1 为止。

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use crate::openpal3::assis_lang;
use crate::openpal3::states::killflyer::{
    FIELD_H, FIELD_W, FLY_DRAW, FlyGame, FlyState, cost_from_hp,
};
use crate::scripting::sce::commands::appraise::{overlay, virtual_mouse};
use crate::scripting::sce::commands::encamp::{draw_rotated_sprite, draw_sprite};
use crate::scripting::sce::{SceCommand, SceState};
use common::store_ext::StoreExt2;
use crosscom::ComRc;
use imgui::{MouseButton, Ui};
use radiance::audio::AudioMemorySource;
use radiance::comdef::ISceneManager;
use radiance::input::Key;
use radiance::rendering::Sprite;

const DIR: &str = "/basedata/basedata/SubGame/KillFlyer";
const LANG_PATH: &str = "/basedata/basedata/AssisLang.lang";
/// AssisLang 里的「减血」「打死苍蝇」「获得金钱」。
const LANG_COST: usize = 243;
const LANG_KILLED: usize = 258;
const LANG_EARNED: usize = 259;
/// 字色（`gbColorQuad(31, 251, 255, 255)`）。
const TEXT: [f32; 4] = [31. / 255., 251. / 255., 1., 1.];
/// 截图的不透明度（`RenderIntercept`：`gbColorQuad(255, 255, 255, 62)`）。
const BACKDROP_ALPHA: f32 = 62. / 255.;
/// 「减血」那一行：(300, 158) 168×32，闪 2.2 秒。
const COST_RECT: (i32, i32, i32, i32) = (300, 158, 468, 190);
const COST_SECS: f32 = 2.2;
/// 闪烁：每帧 alpha 降 7、升 6，在 6..255 之间（`SetBBBSpeed(6.6)` 加到整数 alpha 上截断）。
const FLASH_DOWN: f32 = 7.;
const FLASH_UP: f32 = 6.;
const FLASH_MIN: f32 = 6.;
/// 说明框：(300, 158) 132×132，0.5 秒展开（`UIStaticFlex::DURATION`）。
const NOTE_RECT: (i32, i32, i32, i32) = (300, 158, 432, 290);
const NOTE_OPEN_SECS: f32 = 0.5;
/// 拍子图的大小（`killW`, `killH`）和拍下那张保持多久（`m_time.Wait(100)`）。
const KILL_W: i32 = 64;
const KILL_H: i32 = 58;
const PA_SECS: f32 = 0.1;
/// 嗡嗡声、拍子声（`UISoundDefine.h`）。
const SND_FLYER: &str = "wa039";
const SND_PA: &str = "wa040";
/// 原版按帧算的量按每秒 60 帧折算 [推断]。
const FRAMES_PER_SEC: f32 = 60.;

struct Art {
    normal: Option<Rc<Sprite>>,
    gold: Option<Rc<Sprite>>,
    dead: Option<Rc<Sprite>>,
    kill: Option<Rc<Sprite>>,
    kill_pa: Option<Rc<Sprite>>,
    note: Vec<Option<Rc<Sprite>>>,
    lang: Vec<String>,
}

/// 贴图、声音、计时；命令复制出去时不带（只在虚拟机里跑一份）。
#[derive(Default)]
struct Live {
    art: Option<Art>,
    buzz: Option<Rc<RefCell<Box<dyn AudioMemorySource>>>>,
    clock: f32,
    flash: f32,
    flash_up: bool,
    pa: f32,
    note_open: f32,
}

impl Clone for Live {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl std::fmt::Debug for Live {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Live").finish()
    }
}

#[derive(Debug, Clone)]
pub struct SceCommandMiniGameSwatAFly {
    game: Option<FlyGame>,
    /// 开局时按当时的精算出来的花费（`ftol(精 × 0.01)`，至少 1）。
    cost: i32,
    settled: bool,
    live: Live,
}

impl SceCommandMiniGameSwatAFly {
    pub fn new() -> Self {
        Self {
            game: None,
            cost: 0,
            settled: false,
            live: Live {
                flash: 255.,
                ..Live::default()
            },
        }
    }

    fn start(&mut self, state: &mut SceState) {
        let hp = state
            .global_state()
            .persistent_state()
            .role(0)
            .map(|r| r.hp)
            .unwrap_or(0);
        self.cost = cost_from_hp(hp);
        let seed = state.global_state().persistent_state().money() as u32
            ^ (hp as u32).rotate_left(13)
            ^ 0x1234_5678;
        self.game = Some(FlyGame::new(seed));

        let asset_mgr = state.asset_mgr().clone();
        let sprite = |name: &str| asset_mgr.load_sprite(PathBuf::from(format!("{DIR}/{name}")));
        let lang = asset_mgr
            .vfs()
            .read_to_end_from_gbk(PathBuf::from(LANG_PATH))
            .map(|t| assis_lang::parse_section(&t, "sec1"))
            .unwrap_or_default();
        self.live.art = Some(Art {
            normal: sprite("Flyer_nor.tga"),
            gold: sprite("Flyer_gold.tga"),
            dead: sprite("FlyerDie.tga"),
            kill: sprite("Kill.tga"),
            kill_pa: sprite("Kill_pa.tga"),
            note: (0..9)
                .map(|i| {
                    asset_mgr.load_sprite(PathBuf::from(format!(
                        "/basedata/basedata/ui/flex/note{i}.tga"
                    )))
                })
                .collect(),
            lang,
        });
        // `DispatchFlyer`：有苍蝇就放嗡嗡声。
        if self.game.as_ref().is_some_and(|g| !g.flies.is_empty()) {
            self.live.buzz = start_loop(state, SND_FLYER);
        }
    }

    fn stop_buzz(&mut self, state: &mut SceState) {
        if let Some(source) = self.live.buzz.take() {
            source.borrow_mut().stop();
            state.global_state_mut().remove_sound_source(source);
        }
    }

    /// `KF_Sys::GameOver`：钱加上这一局挣的，景天的精减掉花费（下限 1）。
    fn settle(&mut self, state: &mut SceState) {
        if self.settled {
            return;
        }
        self.settled = true;
        self.stop_buzz(state);
        let earned = self.game.as_ref().map(|g| g.earned).unwrap_or(0);
        let mut p_state = state.global_state_mut().persistent_state_mut();
        p_state.add_money(earned);
        if let Some(role) = p_state.role_mut(0) {
            role.hp = (role.hp - self.cost).max(1);
        }
        log::info!("KillFlyer: 挣了 {earned} 文，花了 {} 精", self.cost);
    }

    fn lang(&self, n: usize) -> &str {
        self.live
            .art
            .as_ref()
            .and_then(|a| a.lang.get(n))
            .map_or("", |s| s.as_str())
    }

    fn draw(&self, ui: &Ui, mouse: (f32, f32), done: bool) {
        let (Some(game), Some(art)) = (self.game.as_ref(), self.live.art.as_ref()) else {
            return;
        };
        let cost_text = format!("{}{}", self.lang(LANG_COST), self.cost);
        let note_text = [
            format!(" {} {} 只", self.lang(LANG_KILLED), game.killed),
            format!(" {} {} ", self.lang(LANG_EARNED), game.earned),
        ];
        overlay(ui, "KillFlyer", |ui, at, _scale| {
            let list = ui.get_window_draw_list();
            // 暗幕盖满整个窗口（宽屏两侧在 800×600 之外也要盖）。
            list.add_rect(
                [0., 0.],
                ui.io().display_size,
                [0., 0., 0., 1. - BACKDROP_ALPHA],
            )
            .filled(true)
            .build();

            // 苍蝇：飞的时候转向飞的方向（`SetRot`，逆时针为正），拍中换死苍蝇的图。
            for fly in &game.flies {
                let sprite = match fly.state {
                    FlyState::Flying if fly.gold => art.gold.as_deref(),
                    FlyState::Flying => art.normal.as_deref(),
                    FlyState::Falling => art.dead.as_deref(),
                    FlyState::Gone | FlyState::Done => continue,
                };
                let (l, t) = (
                    (fly.x - FLY_DRAW * 0.5) as i32,
                    (fly.y - FLY_DRAW * 0.5) as i32,
                );
                let r = (l, t, l + FLY_DRAW as i32, t + FLY_DRAW as i32);
                draw_rotated_sprite(&list, sprite, r, fly.angle, &at);
            }

            // 「减血N」：开局 2.2 秒里闪。
            if self.live.clock < COST_SECS && self.cost > 0 {
                let a = self.live.flash / 255.;
                list.add_text(
                    at(COST_RECT.0, COST_RECT.1 + 8),
                    [TEXT[0], TEXT[1], TEXT[2], a],
                    &cost_text,
                );
            }

            if done {
                // 说明框：从中心 32×32 余弦缓动展开（`UIStaticFlex`，`UpdateVal`），展开完才写字。
                let x = (self.live.note_open / NOTE_OPEN_SECS).min(1.);
                let k = (1. - (x * std::f32::consts::PI).cos()) * 0.5;
                let (cx, cy) = (
                    (NOTE_RECT.0 + NOTE_RECT.2) / 2,
                    (NOTE_RECT.1 + NOTE_RECT.3) / 2,
                );
                let lerp = |a: i32, b: i32| (a as f32 * (1. - k) + b as f32 * k) as i32;
                let r = (
                    lerp(cx - 16, NOTE_RECT.0),
                    lerp(cy - 16, NOTE_RECT.1),
                    lerp(cx + 16, NOTE_RECT.2),
                    lerp(cy + 16, NOTE_RECT.3),
                );
                draw_flex(&list, &art.note, r, &at);
                if x >= 1. {
                    // `"\n\n %s %d 只\n %s %d"`：前两行空。
                    for (k, line) in note_text.iter().enumerate() {
                        let y = NOTE_RECT.1 + 8 + (2 + k as i32) * 18;
                        list.add_text(at(NOTE_RECT.0 + 4, y), TEXT, line);
                    }
                }
            } else {
                // 拍子跟着鼠标（左上角在鼠标 −(32, 29)，64×58），只在场地里画。
                let (mx, my) = (mouse.0 as i32, mouse.1 as i32);
                if (0..FIELD_W as i32).contains(&mx) && (0..FIELD_H as i32).contains(&my) {
                    let (l, t) = (mx - KILL_W / 2, my - KILL_H / 2);
                    let r = (l, t, l + KILL_W, t + KILL_H);
                    let sprite = if self.live.pa > 0. {
                        art.kill_pa.as_deref()
                    } else {
                        art.kill.as_deref()
                    };
                    draw_sprite(&list, sprite, r, &at);
                }
            }
        });
    }
}

/// 九宫格（`UIStaticFlex::Render`）：角块边长取 min(贴图边长, 半宽, 半高)，0..8 是左上、左下、右下、
/// 右上、左、下、右、上、中。
pub(crate) fn draw_flex(
    list: &imgui::DrawListMut<'_>,
    pieces: &[Option<Rc<Sprite>>],
    r: (i32, i32, i32, i32),
    at: &dyn Fn(i32, i32) -> [f32; 2],
) {
    let tile = pieces
        .first()
        .and_then(|p| p.as_ref())
        .map_or(32, |p| p.width() as i32);
    let w = tile.min((r.2 - r.0) / 2);
    let h = tile.min((r.3 - r.1) / 2);
    let rects = [
        (r.0, r.1, r.0 + w, r.1 + h),
        (r.0, r.3 - h, r.0 + w, r.3),
        (r.2 - w, r.3 - h, r.2, r.3),
        (r.2 - w, r.1, r.2, r.1 + h),
        (r.0, r.1 + h, r.0 + w, r.3 - h),
        (r.0 + w, r.3 - h, r.2 - w, r.3),
        (r.2 - w, r.1 + h, r.2, r.3 - h),
        (r.0 + w, r.1, r.2 - w, r.1 + h),
        (r.0 + w, r.1 + h, r.2 - w, r.3 - h),
    ];
    for (k, q) in rects.iter().enumerate() {
        let Some(tex) = pieces.get(k).and_then(|p| p.as_ref()) else {
            continue;
        };
        if q.0 < q.2 && q.1 < q.3 {
            list.add_image(tex.imgui_texture_id(), at(q.0, q.1), at(q.2, q.3))
                .build();
        }
    }
}

/// 循环放一个音效（`SoundMgr::PlaySnd(name, 0)`）。
fn start_loop(state: &mut SceState, name: &str) -> Option<Rc<RefCell<Box<dyn AudioMemorySource>>>> {
    let data = state.asset_mgr().load_snd_data(name).ok()?;
    let mut source = state.audio_engine().create_source();
    source.set_data(data, radiance::audio::Codec::Wav);
    source.play(true);
    let source = Rc::new(RefCell::new(source));
    state.global_state_mut().add_sound_source(source.clone());
    Some(source)
}

impl SceCommand for SceCommandMiniGameSwatAFly {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        if self.game.is_none() {
            self.start(state);
        }
        if state.fast_forward() {
            self.settle(state);
            return true;
        }

        // [移植] 手柄 B / Esc 随时离开（原版右键在场地外才行，而场地是整屏）。
        let quit = state.input().get_key_state(Key::Escape).pressed()
            || state.input().get_key_state(Key::GamePadSouth).pressed();

        let mouse = virtual_mouse(ui);
        let in_field = (0. ..FIELD_W).contains(&mouse.0) && (0. ..FIELD_H).contains(&mouse.1);
        let click = ui.is_mouse_clicked(MouseButton::Left);
        let done = self.game.as_ref().map(|g| g.finished()).unwrap_or(true);

        self.live.clock += delta_sec;
        let frames = delta_sec * FRAMES_PER_SEC;
        if self.live.flash_up {
            self.live.flash += FLASH_UP * frames;
            if self.live.flash >= 255. {
                self.live.flash = 255.;
                self.live.flash_up = false;
            }
        } else {
            self.live.flash -= FLASH_DOWN * frames;
            if self.live.flash <= FLASH_MIN {
                self.live.flash = FLASH_MIN;
                self.live.flash_up = true;
            }
        }
        self.live.pa = (self.live.pa - delta_sec).max(0.);

        if !done {
            if click && in_field {
                self.live.pa = PA_SECS;
                crate::scripting::sce::commands::play_sfx(state, SND_PA);
                if let Some(game) = self.game.as_mut() {
                    game.swat(mouse.0, mouse.1);
                }
            }
            if let Some(game) = self.game.as_mut() {
                game.tick(delta_sec);
            }
            // 场地里藏起鼠标，拍子代替它。
            if in_field {
                ui.set_mouse_cursor(None);
            }
            // 苍蝇全没了（拍中掉下去的还在掉也算）就停嗡嗡声（原版拍中最后一只时就停）。
            let flying = self
                .game
                .as_ref()
                .is_some_and(|g| g.flies.iter().any(|f| f.state == FlyState::Flying));
            if !flying {
                self.stop_buzz(state);
            }
        } else {
            self.stop_buzz(state);
            self.live.note_open += delta_sec;
        }
        let done = self.game.as_ref().map(|g| g.finished()).unwrap_or(true);
        self.draw(ui, mouse, done);

        // 说明框展开、字出来之后（`m_isExitKF`）在场地里左键离开。
        let note_ready = done && self.live.note_open >= NOTE_OPEN_SECS;
        if quit || (note_ready && click && in_field) {
            self.settle(state);
            return true;
        }
        false
    }
}
