//! 104 `APPREntry` —— 鉴定小游戏。规则在 [`crate::openpal3::appraise`]，
//! 逆向笔记在 `switch/re/notes/appraise_minigame.md`。
//!
//! 原版是 `SwitchMode` 进一个独立模式，状态对象分四个阶段：询问 → Exhibit
//! 展示 → Do 鉴定 → 退出。脚本停在 104 上等它结束，108 `GetAppr` 再读
//! `DAT_00d92784`。这里做成一条阻塞指令：`initialize` 往场景栈上压一个
//! [`CoreScene`]，里面放背景贴图片和十个 pol 文物；每帧按窗口尺寸把相机摆到
//! (0, 0, D) 看原点，让 z = 0 平面上 1 世界单位 = 800×600 虚拟屏幕里的
//! 1 像素，所以原版控件的像素坐标能直接当世界坐标用。按钮、数字、文字走
//! imgui，画在 3D 之上。
//!
//! 文物的摆放、大小和转速照原版的 3D 控件来（正交相机，见
//! [`layout::View3d`]），不是按格子塞。界面上每个元件的矩形也都是从
//! `FUN_00528650` / `FUN_005257b0` 的指令里读出来的，包括贴图在 2 的幂
//! 画布里占哪一块（见 [`layout`] 开头）。
//!
//! 和原版不一样的两处，都是移植自己加的，不是没查到：放大视图里可以拖动
//! 转模型（原版两个放大控件的 `+0xec` 是 0，模型不动，也没有写转角的调用），
//! 以及数字键 1–5 / Esc / 回车这些键盘操作。

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use common::store_ext::StoreExt2;
use crosscom::ComRc;
use imgui::{Condition, MouseButton, Ui};
use mini_fs::StoreExt;
use radiance::audio::Codec;
use radiance::comdef::{IEntity, IEntityExt, ISceneExt, ISceneManager, IStaticMeshComponent};
use radiance::components::mesh::{Geometry, StaticMeshComponent, TexCoord};
use radiance::imgui::{GameFontSize, game_font};
use radiance::math::Vec3;
use radiance::rendering::{BlendMode, SimpleMaterialDef, Sprite};
use radiance::scene::{CoreEntity, CoreScene, SceneLighting};

use crate::openpal3::appraise::{
    self, ASSIS_LANG_PATH, ASSIS_LANG_SECTION, DFT_PATH, DFT_SECTION, DO_DIR, EXHIBIT_DIR,
    ITEM_COUNT, MODEL_DIR, MUSIC, NUMBER_DIR, PAIRS, PickError, Round, lang,
    layout::{self, Rect},
    sfx,
};
use crate::openpal3::assis_lang;
use crate::scripting::sce::{SceCommand, SceState};

/// 背景片放在 z = −ZB，按 (D + ZB) / D 放大，看起来就和贴在 z = 0 一样，
/// 但文物总在它前面。
const BACKDROP_DEPTH: f32 = 500.;
/// 自转速度（弧度/秒）。原版是 58 度/秒，见 [`layout::SPIN_DEG_PER_SEC`]。
const SPIN_SPEED: f32 = layout::SPIN_DEG_PER_SEC * std::f32::consts::PI / 180.;
/// 拖动转模型：每像素多少弧度。**原版没有这个操作**——两个放大视图控件
/// （0xaa0b09 / 0xaa0c27）建好以后只调了 `FUN_0043a070`（尺度）和
/// `FUN_00439850`（位置），没调 `FUN_0043a090`，所以自转模式 `+0xec` 是 0；
/// 直接写转角的 `FUN_0043a0e0` 全程只有三个调用点，都不在
/// 0x525000–0x529500。移植加的，速度随手定。
const DRAG_SPEED: f32 = 0.012;

pub(crate) const WHITE: [f32; 4] = [1., 1., 1., 1.];
/// 对话框文字色 0x1ffbff（原版 `FUN_00525841` 一带的 0xfffb1f/0x1ffbff）。
pub(crate) const TEXT_COLOR: [f32; 4] = [1., 0.98, 0.12, 1.];
pub(crate) const PANEL_COLOR: [f32; 4] = [0.05, 0.05, 0.1, 0.9];
pub(crate) const HIGHLIGHT: [f32; 4] = [1., 0.92, 0.45, 1.];

pub struct SceCommandAppraise {
    game: Option<Box<AppraiseGame>>,
    /// 第一次 update 才建场景（要用 `ui` 和场景栈）。
    started: bool,
}

impl Clone for SceCommandAppraise {
    fn clone(&self) -> Self {
        Self {
            game: None,
            started: false,
        }
    }
}

impl std::fmt::Debug for SceCommandAppraise {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SceCommandAppraise").finish()
    }
}

impl SceCommandAppraise {
    pub fn new() -> Self {
        Self {
            game: None,
            started: false,
        }
    }
}

impl SceCommand for SceCommandAppraise {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        if !self.started {
            self.started = true;
            // 跑图时不玩：等于开头选「否」，结果字节保持 0。
            if !state.fast_forward() {
                match AppraiseGame::load(state) {
                    Some(game) => {
                        scene_manager.push_scene(game.scene.clone());
                        self.game = Some(Box::new(game));
                    }
                    None => log::error!("鉴定小游戏的素材读不出来，跳过"),
                }
            }
            state.global_state_mut().set_appraise_result(0);
        }

        let Some(game) = self.game.as_mut() else {
            return true;
        };
        game.update(ui, state, delta_sec);
        if game.phase == Phase::Done {
            game.leave(state);
            scene_manager.pop_scene();
            self.game = None;
            return true;
        }
        false
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Phase {
    /// Lang081「要玩鉴定游戏吗？」
    Ask,
    /// 展示页；`zoom` 是正在放大看的那件。
    Exhibit {
        zoom: Option<usize>,
    },
    /// 鉴定页。
    Do,
    /// 提示框（答错的说明、结算），点一下关闭后进 `next`。
    Note {
        text: String,
        next: Box<Phase>,
    },
    /// 结算框关掉之后：发奖、退出。
    Finish,
    Done,
}

/// Do 页里放大镜正被拖着。
#[derive(Debug, Clone, Copy)]
struct Drag {
    magnifier: usize,
}

struct Model {
    entity: ComRc<IEntity>,
}

struct Backdrop {
    entity: ComRc<IEntity>,
}

struct AppraiseGame {
    scene: ComRc<radiance::comdef::IScene>,
    phase: Phase,
    round: Round,
    /// AssisLang 表。
    lang: Vec<String>,
    /// 五件的鉴别要点 `Diffent0..4`。
    diffent: Vec<String>,
    /// 五对的 (真品名, 赝品名)。
    names: [(String, String); ITEM_COUNT],
    /// 五对的 [真品, 赝品] 模型，读不出来的是 `None`。
    models: Vec<[Option<Model>; 2]>,
    exhibit_bk: Option<Backdrop>,
    exhibit_zoom: Option<Backdrop>,
    doing_bk: Option<Backdrop>,
    do_zoom: Option<Backdrop>,
    // imgui 层的贴图
    do_button: Option<Rc<Sprite>>,
    remark: Option<Rc<Sprite>>,
    queue: [Option<Rc<Sprite>>; 2],
    un_queue: Option<Rc<Sprite>>,
    magnifiers: [Option<Rc<Sprite>>; 3],
    un_magnifier: Option<Rc<Sprite>>,
    result: [Option<Rc<Sprite>>; 2],
    numbers: Vec<Option<Rc<Sprite>>>,
    time_bar: [Option<Rc<Sprite>>; 2],

    /// 展示/放大视图里的自转角。
    spin: f32,
    /// Do 页当前放大的是左 (0) / 右 (1)。
    do_zoom_side: Option<usize>,
    /// Do 页里模型被拖出来的转角。
    do_angle: f32,
    drag: Option<Drag>,
    magnifier_used: [bool; 3],
    /// Result0 / Result1：选了左边还是右边。
    chosen: Option<bool>,
    /// 计时器累积的秒。
    timer_acc: f32,
    /// 结算文字里要不要列奖品（`+0x6cc`，从冒险模式进来）。
    from_adventure: bool,
    music_started: bool,
}

impl AppraiseGame {
    fn load(state: &SceState) -> Option<Self> {
        let asset_mgr = state.asset_mgr().clone();
        let vfs = asset_mgr.vfs();
        let factory = asset_mgr.component_factory();

        let lang = vfs
            .read_to_end_from_gbk(Self::vfs_path(ASSIS_LANG_PATH))
            .map(|t: String| assis_lang::parse_section(&t, ASSIS_LANG_SECTION))
            .unwrap_or_default();
        let diffent = vfs
            .read_to_end_from_gbk(Self::vfs_path(DFT_PATH))
            .map(|t: String| assis_lang::parse_section(&t, DFT_SECTION))
            .unwrap_or_default();
        if lang.len() <= lang::FAILED {
            log::warn!("AssisLang.lang 只读到 {} 条", lang.len());
        }

        let db = state.global_state().game_db()?.clone();
        let item_text = |id: u32| -> (String, String) {
            let Some(item) = db.item(id) else {
                return (String::new(), String::new());
            };
            (
                item.name.as_str().unwrap_or_default(),
                item.model_name.as_str().unwrap_or_default(),
            )
        };

        let scene = CoreScene::create();
        scene.set_lighting(SceneLighting::new([1., 1., 1.], vec![]));

        let mut names: [(String, String); ITEM_COUNT] = Default::default();
        let mut models = Vec::with_capacity(ITEM_COUNT);
        for (i, (real, fake)) in PAIRS.iter().enumerate() {
            let (real_name, real_model) = item_text(*real);
            let (fake_name, fake_model) = item_text(*fake);
            names[i] = (real_name, fake_name);
            let mut pair = [None, None];
            for (k, model_name) in [real_model, fake_model].iter().enumerate() {
                let path = Self::vfs_path(&format!("{MODEL_DIR}/{model_name}.pol"));
                if vfs.open(&path).is_err() {
                    log::warn!("鉴定文物模型不存在：{}", path.display());
                    continue;
                }
                let entity = crate::openpal3::loaders::pol::create_entity_from_pol_model(
                    &factory,
                    vfs,
                    &path,
                    format!("APPRAISE_{i}_{k}"),
                    false,
                );
                scene.add_entity(entity.clone());
                pair[k] = Some(Model { entity });
            }
            models.push(pair);
        }
        if models.iter().flatten().all(|m| m.is_none()) {
            return None;
        }

        let backdrop = |name: &str, rect: layout::Rect| -> Option<Backdrop> {
            let image = asset_mgr.read_picture(Self::vfs_path(name))?;
            let entity = Self::quad(&factory, name, image, rect);
            entity.set_visible(false);
            scene.add_entity(entity.clone());
            Some(Backdrop { entity })
        };
        let exhibit_bk = backdrop(&format!("{EXHIBIT_DIR}/ExhibitBk.tga"), layout::EXHIBIT_BK);
        let exhibit_zoom = backdrop(
            &format!("{EXHIBIT_DIR}/ExhibitZoom.tga"),
            layout::EXHIBIT_ZOOM_BK,
        );
        let doing_bk = backdrop(&format!("{DO_DIR}/doingBk.tga"), layout::DO_BK);
        let do_zoom = backdrop(&format!("{DO_DIR}/ExhibitZoom.tga"), layout::DO_ZOOM_BK);

        // 贴图都是左下角为原点的 `.tga`，画面贴在画布左下角、右上补透明到
        // 2 的幂，控件矩形的宽高就是画面本身的大小（见 [`layout`] 开头）。
        // 所以按控件矩形从画布左下角裁一块出来再做 sprite，不然 imgui 会把
        // 补出来的空白一起拉伸进去。
        let sprite = |name: String, rect: layout::Rect| -> Option<Rc<Sprite>> {
            let mut image = asset_mgr.read_picture(Self::vfs_path(&name))?;
            let (x, y, w, h) = layout::art_in_canvas(image.height(), rect);
            let w = w.min(image.width());
            let view = image::imageops::crop(&mut image, x, y, w, h).to_image();
            Some(Rc::new(Sprite::load_from_image(
                view,
                asset_mgr.component_factory().as_ref(),
            )))
        };
        let numbers = (0..10)
            .map(|n| sprite(format!("{NUMBER_DIR}/{n}.tga"), layout::SECONDS))
            .collect();

        Some(Self {
            scene,
            phase: Phase::Ask,
            round: Round::new(rand::random::<u8>() as usize & 1),
            lang,
            diffent,
            names,
            models,
            exhibit_bk,
            exhibit_zoom,
            doing_bk,
            do_zoom,
            do_button: sprite(format!("{EXHIBIT_DIR}/Do0.tga"), layout::EXHIBIT_DO_BUTTON),
            remark: sprite(format!("{EXHIBIT_DIR}/ExhibitRemark.tga"), layout::REMARK),
            queue: [
                sprite(format!("{DO_DIR}/queue0.tga"), layout::queue_button(0)),
                sprite(format!("{DO_DIR}/queue1.tga"), layout::queue_button(0)),
            ],
            un_queue: sprite(format!("{DO_DIR}/unQueue.tga"), layout::queue_button(0)),
            magnifiers: [
                sprite(format!("{DO_DIR}/Magnifier0.tga"), layout::magnifier(0)),
                sprite(format!("{DO_DIR}/Magnifier1.tga"), layout::magnifier(0)),
                sprite(format!("{DO_DIR}/Magnifier2.tga"), layout::magnifier(0)),
            ],
            un_magnifier: sprite(format!("{DO_DIR}/unMagnifier.tga"), layout::magnifier(0)),
            result: [
                sprite(format!("{DO_DIR}/Result0.tga"), layout::RESULT0),
                sprite(format!("{DO_DIR}/Result1.tga"), layout::RESULT0),
            ],
            numbers,
            time_bar: [
                sprite(format!("{DO_DIR}/TimeBarbk.tga"), layout::TIME_BAR),
                sprite(format!("{DO_DIR}/TimeBarFo.tga"), layout::TIME_BAR),
            ],
            spin: 0.,
            do_zoom_side: None,
            do_angle: 0.,
            drag: None,
            magnifier_used: [false; 3],
            chosen: None,
            timer_acc: 0.,
            from_adventure: true,
            music_started: false,
        })
    }

    fn vfs_path(rel: &str) -> PathBuf {
        PathBuf::from("/basedata/basedata").join(rel)
    }

    /// 一张背景片：盖住控件矩形 `rect`，放在 z = 0。
    ///
    /// uv 不是整张图：`.tga` 是左下角为原点的，画面贴在 2 的幂画布的
    /// **左下角**，右边和上边补透明。控件矩形的宽高就是画面的大小，所以
    /// u 取 `0..宽/画布宽`、v（自上而下）取 `1−高/画布高..1`。四张背景
    /// 都是 1024×512 的画布装 626×466 左右的画面，对得上。
    fn quad(
        factory: &Rc<dyn radiance::rendering::ComponentFactory>,
        name: &str,
        image: image::RgbaImage,
        rect: layout::Rect,
    ) -> ComRc<IEntity> {
        let (tw, th) = (image.width() as f32, image.height() as f32);
        let (x0, y0) = Self::to_world(rect.0 as f32, rect.1 as f32);
        let (x1, y1) = Self::to_world(rect.2 as f32, rect.3 as f32);
        let u1 = ((rect.2 - rect.0) as f32 / tw).min(1.);
        let v0 = 1. - ((rect.3 - rect.1) as f32 / th).min(1.);
        let verts = vec![
            Vec3::new(x0, y0, 0.),
            Vec3::new(x1, y0, 0.),
            Vec3::new(x1, y1, 0.),
            Vec3::new(x0, y1, 0.),
        ];
        let texcoords = vec![
            TexCoord::new(0., v0),
            TexCoord::new(u1, v0),
            TexCoord::new(u1, 1.),
            TexCoord::new(0., 1.),
        ];
        // 两面都画，免得绕向和背面剔除对不上。
        let indices: Vec<u32> = vec![0, 1, 2, 0, 2, 3, 0, 2, 1, 0, 3, 2];
        let material = SimpleMaterialDef::create_with_image(name, Some(image))
            .with_blend(BlendMode::AlphaBlend)
            .make_unique();
        let geometry = Geometry::new(
            &verts,
            None,
            std::slice::from_ref(&texcoords),
            indices,
            material,
        );
        let entity = CoreEntity::create(format!("APPRAISE_BK_{name}"), true);
        let mesh = StaticMeshComponent::new(entity.clone(), vec![geometry], factory.clone());
        entity.add_component(IStaticMeshComponent::uuid(), ComRc::from_object(mesh));
        entity
    }

    /// 虚拟屏幕像素 → z = 0 平面上的世界坐标。
    fn to_world(x: f32, y: f32) -> (f32, f32) {
        (
            x - layout::SCREEN_W as f32 * 0.5,
            layout::SCREEN_H as f32 * 0.5 - y,
        )
    }

    /// 每帧把相机摆到能让 z = 0 平面 1 单位 = 1 虚拟像素的距离上，背景片
    /// 跟着放大。
    fn place_camera(&self, ui: &Ui) {
        let [sw, sh] = ui.io().display_size;
        if sw <= 0. || sh <= 0. {
            return;
        }
        let scale = (sw / layout::SCREEN_W as f32).min(sh / layout::SCREEN_H as f32);
        let half_height = sh / (2. * scale);
        let aspect = sw / sh;
        let mut camera = self.scene.camera_mut();
        let fov43 = camera.fov43();
        let fov = if aspect > 1. {
            fov43 / 4. * 3. * aspect
        } else {
            fov43
        };
        let fti = 1. / (fov / 2.).tan();
        let m22 = if aspect < 1. { fti } else { fti * aspect };
        let distance = half_height * m22;
        camera
            .transform_mut()
            .set_position(&Vec3::new(0., 0., distance))
            .look_at(&Vec3::new(0., 0., 0.));
        drop(camera);

        let k = (distance + BACKDROP_DEPTH) / distance;
        for bk in [
            &self.exhibit_bk,
            &self.exhibit_zoom,
            &self.doing_bk,
            &self.do_zoom,
        ]
        .into_iter()
        .flatten()
        {
            bk.entity
                .transform()
                .borrow_mut()
                .set_position(&Vec3::new(0., 0., -BACKDROP_DEPTH))
                .scale_local(&Vec3::new(k, k, 1.));
        }
    }

    fn show_backdrop(&self, which: Option<&Backdrop>) {
        for bk in [
            &self.exhibit_bk,
            &self.exhibit_zoom,
            &self.doing_bk,
            &self.do_zoom,
        ]
        .into_iter()
        .flatten()
        {
            let on = which.is_some_and(|w| w.entity.name() == bk.entity.name());
            bk.entity.set_visible(on);
        }
    }

    fn hide_models(&self) {
        for m in self.models.iter().flatten().flatten() {
            m.entity.set_visible(false);
        }
    }

    /// 把一件文物摆到虚拟屏幕 (cx, cy) 处，最长边缩到 `fit` 像素，绕 y 转 `angle`。
    /// 照原版摆一件文物：模型按它自己的世界坐标放在 `view` 这台正交相机下，
    /// **不重新居中、也不按格子缩放**——在屏幕上多大完全由 `OrthoSize` 决定
    /// （`SCREEN_W / (2·ortho)` 个虚拟像素每世界单位）。
    fn place_model(
        &self,
        pair: usize,
        which: usize,
        view: &layout::View3d,
        world: (f32, f32),
        angle: f32,
    ) {
        let Some(m) = self.models.get(pair).and_then(|p| p[which].as_ref()) else {
            return;
        };
        let (sx, sy) = view.to_screen(world.0, world.1);
        let (wx, wy) = Self::to_world(sx, sy);
        let k = view.scale();
        m.entity.set_visible(true);
        m.entity
            .transform()
            .borrow_mut()
            .set_position(&Vec3::new(wx, wy, 0.))
            .rotate_axis_angle_local(&Vec3::UP, angle)
            .scale_local(&Vec3::new(k, k, k));
    }

    fn text(&self, i: usize) -> &str {
        self.lang.get(i).map(String::as_str).unwrap_or("")
    }

    fn update(&mut self, ui: &Ui, state: &mut SceState, delta_sec: f32) {
        self.place_camera(ui);
        self.spin = (self.spin + SPIN_SPEED * delta_sec) % std::f32::consts::TAU;

        let phase = self.phase.clone();
        match phase {
            Phase::Ask => self.update_ask(ui, state),
            Phase::Exhibit { zoom } => self.update_exhibit(ui, state, zoom),
            Phase::Do => self.update_do(ui, state, delta_sec),
            Phase::Note { text, next } => self.update_note(ui, &text, *next),
            Phase::Finish => {
                // 结算框关掉：答对的那几对的赝品各加一件（`FUN_0040ccf0`）。
                if self.from_adventure {
                    for id in self.round.rewards() {
                        state
                            .global_state_mut()
                            .persistent_state_mut()
                            .add_item(id, 1);
                    }
                }
                state
                    .global_state_mut()
                    .set_appraise_result(self.round.result as i32);
                self.phase = Phase::Done;
            }
            Phase::Done => {}
        }
    }

    fn leave(&mut self, state: &mut SceState) {
        if self.music_started {
            state.global_state_mut().bgm_source().stop();
            state.global_state_mut().play_default_bgm();
        }
    }

    // ---------- 阶段 0：询问 ----------

    fn update_ask(&mut self, ui: &Ui, state: &mut SceState) {
        self.show_backdrop(None);
        self.hide_models();
        if !self.music_started {
            // 原版询问框弹出时放 wa003；音乐在阶段 1 才起。
            play_sfx(state, sfx::ASK);
            self.music_started = true;
            state.global_state_mut().play_bgm(MUSIC);
        }

        let (yes_rect, no_rect) = (layout::ASK_YES, layout::ASK_NO);
        let mouse = virtual_mouse(ui);
        let question = self.text(lang::ASK).to_string();
        overlay(ui, "AppraiseAsk", |ui, at, scale| {
            let list = ui.get_window_draw_list();
            let r = layout::ASK_BOX;
            list.add_rect(at(r.0, r.1), at(r.2, r.3), PANEL_COLOR)
                .filled(true)
                .build();
            list.add_rect(at(r.0, r.1), at(r.2, r.3), HIGHLIGHT)
                .thickness(1.)
                .build();
            let _font = game_font(GameFontSize::LARGE).map(|f| ui.push_font(f));
            list.add_text(at(r.0 + 24, r.1 + 20), TEXT_COLOR, &question);
            for (rect, label) in [(yes_rect, "是"), (no_rect, "否")] {
                let hovered = layout::contains(rect, mouse.0, mouse.1);
                list.add_rect(
                    at(rect.0, rect.1),
                    at(rect.2, rect.3),
                    if hovered { HIGHLIGHT } else { WHITE },
                )
                .thickness(1.)
                .build();
                let size = ui.calc_text_size(label);
                let (cx, cy) = layout::center(rect);
                let p = at(cx as i32, cy as i32);
                list.add_text(
                    [p[0] - size[0] * 0.5, p[1] - size[1] * 0.5],
                    TEXT_COLOR,
                    label,
                );
            }
            let _ = scale;
        });

        let clicked = ui.is_mouse_clicked(MouseButton::Left);
        let yes = (clicked && layout::contains(yes_rect, mouse.0, mouse.1))
            || ui.is_key_pressed(imgui::Key::Enter)
            || ui.is_key_pressed(imgui::Key::Y)
            || ui.is_key_pressed(imgui::Key::Keypad1)
            || ui.is_key_pressed(imgui::Key::Alpha1);
        let no = (clicked && layout::contains(no_rect, mouse.0, mouse.1))
            || ui.is_key_pressed(imgui::Key::Escape)
            || ui.is_key_pressed(imgui::Key::N)
            || ui.is_key_pressed(imgui::Key::Alpha2);
        if yes {
            self.phase = Phase::Exhibit { zoom: None };
        } else if no {
            // 选「否」直接到阶段 3，结果字节保持 0，也不发奖。
            self.phase = Phase::Done;
        }
    }

    // ---------- 阶段 1：展示 ----------

    fn update_exhibit(&mut self, ui: &Ui, state: &mut SceState, zoom: Option<usize>) {
        self.hide_models();
        let mouse = virtual_mouse(ui);
        let clicked = ui.is_mouse_clicked(MouseButton::Left);
        let right = ui.is_mouse_clicked(MouseButton::Right);

        match zoom {
            None => {
                self.show_backdrop(self.exhibit_bk.as_ref());
                // 展示页五件在原版里是持续自转的（控件 0xaa0b08 的
                // `FUN_0043a090(1, …)` → 模式 1），五件共用一台相机、同一个角度。
                for i in 0..layout::EXHIBIT_POS.len() {
                    self.place_model(
                        i,
                        0,
                        &layout::VIEW_EXHIBIT,
                        layout::EXHIBIT_POS[i],
                        self.spin,
                    );
                }
                let do_button = self.do_button.clone();
                let hovered_slot = layout::EXHIBIT_SLOTS
                    .iter()
                    .position(|r| layout::contains(*r, mouse.0, mouse.1));
                let do_hovered = layout::contains(layout::EXHIBIT_DO_BUTTON, mouse.0, mouse.1);
                overlay(ui, "AppraiseExhibit", |ui, at, _| {
                    let list = ui.get_window_draw_list();
                    if let Some(i) = hovered_slot {
                        let r = layout::EXHIBIT_SLOTS[i];
                        list.add_rect(at(r.0, r.1), at(r.2, r.3), HIGHLIGHT)
                            .thickness(2.)
                            .build();
                    }
                    let r = layout::EXHIBIT_DO_BUTTON;
                    match &do_button {
                        Some(s) => list
                            .add_image(s.imgui_texture_id(), at(r.0, r.1), at(r.2, r.3))
                            .build(),
                        None => {
                            let _font = game_font(GameFontSize::LARGE).map(|f| ui.push_font(f));
                            list.add_text(at(r.0 + 30, r.1 + 4), TEXT_COLOR, "鉴定");
                        }
                    }
                    if do_hovered {
                        list.add_rect(at(r.0, r.1), at(r.2, r.3), HIGHLIGHT)
                            .thickness(2.)
                            .build();
                    }
                });

                if clicked {
                    if let Some(i) = hovered_slot {
                        self.spin = 0.;
                        self.phase = Phase::Exhibit { zoom: Some(i) };
                        return;
                    }
                    if do_hovered {
                        self.enter_do(state);
                        return;
                    }
                }
                if ui.is_key_pressed(imgui::Key::Enter) || ui.is_key_pressed(imgui::Key::Space) {
                    self.enter_do(state);
                    return;
                }
                // 数字键 1–5 也能放大看：原版只有鼠标，这是移植加的。
                for (i, key) in [
                    imgui::Key::Alpha1,
                    imgui::Key::Alpha2,
                    imgui::Key::Alpha3,
                    imgui::Key::Alpha4,
                    imgui::Key::Alpha5,
                ]
                .iter()
                .enumerate()
                {
                    if ui.is_key_pressed(*key) {
                        self.spin = 0.;
                        self.phase = Phase::Exhibit { zoom: Some(i) };
                    }
                }
            }
            Some(i) => {
                self.show_backdrop(self.exhibit_zoom.as_ref());
                // 放大视图（控件 0xaa0b09）没设过自转模式，原版是静止的。
                self.place_model(i, 0, &layout::VIEW_EXHIBIT_ZOOM, (0., 0.), 0.);
                let name = self.names[i].0.clone();
                let diffent = self
                    .diffent
                    .get(i)
                    .map(|s| assis_lang::unescape(s))
                    .unwrap_or_default();
                self.draw_remark(ui, &name, &diffent);

                if right
                    || ui.is_key_pressed(imgui::Key::Escape)
                    || (clicked && !layout::contains(layout::ZOOM_PANE, mouse.0, mouse.1))
                {
                    self.phase = Phase::Exhibit { zoom: None };
                }
            }
        }
    }

    /// 说明牌（展示页的放大视图）：牌子 `+0x518`、名字 0xaa0c21、要点
    /// 0xaa0c19，三个矩形都照原版。原版的文字区 [`layout::REMARK_TEXT`]
    /// 比牌子本身高 54 px，长文字会漏到牌子外面，这里照抄。
    fn draw_remark(&self, ui: &Ui, name: &str, body: &str) {
        let remark = self.remark.clone();
        let name = name.to_string();
        let body = body.to_string();
        overlay(ui, "AppraiseRemark", |ui, at, scale| {
            let list = ui.get_window_draw_list();
            let r = layout::REMARK;
            match &remark {
                Some(s) => list
                    .add_image(s.imgui_texture_id(), at(r.0, r.1), at(r.2, r.3))
                    .build(),
                None => list
                    .add_rect(at(r.0, r.1), at(r.2, r.3), PANEL_COLOR)
                    .filled(true)
                    .build(),
            }
            {
                let _font = game_font(GameFontSize::LARGE).map(|f| ui.push_font(f));
                let n = layout::REMARK_NAME;
                list.add_text(at(n.0, n.1), TEXT_COLOR, &name);
            }
            let _font = game_font(GameFontSize::SMALL).map(|f| ui.push_font(f));
            let t = layout::REMARK_DIFFENT;
            let width = (t.2 - t.0) as f32 * scale;
            let mut y = at(t.0, t.1)[1];
            let line_h = ui.text_line_height_with_spacing();
            for line in wrap_lines(ui, &body, width) {
                list.add_text([at(t.0, t.1)[0], y], TEXT_COLOR, &line);
                y += line_h;
            }
        });
    }

    fn enter_do(&mut self, state: &mut SceState) {
        let _ = state;
        self.phase = Phase::Do;
        self.do_zoom_side = None;
        self.do_angle = 0.;
        self.chosen = None;
        self.timer_acc = 0.;
    }

    // ---------- 阶段 2：鉴定 ----------

    fn update_do(&mut self, ui: &Ui, state: &mut SceState, delta_sec: f32) {
        self.hide_models();
        let mouse = virtual_mouse(ui);
        let clicked = ui.is_mouse_clicked(MouseButton::Left);
        let released = ui.is_mouse_released(MouseButton::Left);
        let down = ui.is_mouse_down(MouseButton::Left);
        let right = ui.is_mouse_clicked(MouseButton::Right);

        // 计时（`FUN_00526740`：`now − 上次 > 1000` 才走一格；提示框开着
        // （`+0x183c`）或者已经答错（`+0x3abd`）时整段跳过，不走表）
        self.timer_acc += delta_sec;
        while self.timer_acc >= 1. {
            self.timer_acc -= 1.;
            if self.round.timer.tick() {
                self.settle(state);
                return;
            }
        }

        let cur = self.round.current;
        let side = self.round.side;
        // 放大视图里：右键关，拖动转
        if let Some(z) = self.do_zoom_side {
            self.show_backdrop(self.do_zoom.as_ref());
            let which = if z == side { 0 } else { 1 };
            if down && layout::contains(layout::ZOOM_PANE, mouse.0, mouse.1) {
                let [dx, _] = ui.io().mouse_delta;
                let [sw, sh] = ui.io().display_size;
                let scale = (sw / layout::SCREEN_W as f32).min(sh / layout::SCREEN_H as f32);
                self.do_angle += dx / scale.max(1e-3) * DRAG_SPEED;
            }
            self.place_model(cur, which, &layout::VIEW_DO_ZOOM, (0., 0.), self.do_angle);
            if right || ui.is_key_pressed(imgui::Key::Escape) {
                self.do_zoom_side = None;
            }
            self.draw_do_hud(ui, mouse, None);
            return;
        }

        self.show_backdrop(self.doing_bk.as_ref());
        // 真品放世界位置表的第 `side` 项，赝品放另一项；两件都持续自转
        // （控件 0xaa0c25/26 都是模式 1）。
        let real_world = layout::DO_POS[side as usize & 1];
        let fake_world = layout::DO_POS[1 - (side as usize & 1)];
        self.place_model(cur, 0, &layout::VIEW_DO, real_world, self.spin);
        self.place_model(cur, 1, &layout::VIEW_DO, fake_world, self.spin);

        // 拖放大镜
        if let Some(drag) = self.drag {
            if released {
                self.drag = None;
                let target = if layout::contains(layout::DO_LEFT, mouse.0, mouse.1) {
                    Some(0)
                } else if layout::contains(layout::DO_RIGHT, mouse.0, mouse.1) {
                    Some(1)
                } else {
                    None
                };
                if let Some(t) = target {
                    if self.round.use_magnifier() {
                        self.magnifier_used[drag.magnifier] = true;
                        self.do_zoom_side = Some(t);
                    }
                }
            }
        } else if clicked {
            for i in 0..3 {
                if !self.magnifier_used[i]
                    && self.round.magnifiers > 0
                    && layout::contains(layout::magnifier(i as i32), mouse.0, mouse.1)
                {
                    play_sfx(state, sfx::MAGNIFIER);
                    self.drag = Some(Drag { magnifier: i });
                }
            }
        }

        // 队列按钮
        if clicked && self.drag.is_none() {
            for i in 0..ITEM_COUNT {
                if layout::contains(layout::queue_button(i as i32), mouse.0, mouse.1) {
                    self.pick(i, state);
                }
            }
        }
        for (i, key) in [
            imgui::Key::Alpha1,
            imgui::Key::Alpha2,
            imgui::Key::Alpha3,
            imgui::Key::Alpha4,
            imgui::Key::Alpha5,
        ]
        .iter()
        .enumerate()
        {
            if ui.is_key_pressed(*key) {
                self.pick(i, state);
            }
        }

        // 勾选 + 确定
        if clicked && self.drag.is_none() {
            if layout::contains(layout::RESULT0, mouse.0, mouse.1) {
                self.chosen = Some(true);
            } else if layout::contains(layout::RESULT1, mouse.0, mouse.1) {
                self.chosen = Some(false);
            }
        }
        if ui.is_key_pressed(imgui::Key::LeftArrow) {
            self.chosen = Some(true);
        }
        if ui.is_key_pressed(imgui::Key::RightArrow) {
            self.chosen = Some(false);
        }
        let confirm =
            (clicked && self.drag.is_none() && layout::contains(layout::CONFIRM, mouse.0, mouse.1))
                || ui.is_key_pressed(imgui::Key::Enter);

        self.draw_do_hud(ui, mouse, self.drag);

        if confirm {
            if let Some(chose_left) = self.chosen {
                self.judge(chose_left, state);
            }
        }
    }

    fn pick(&mut self, i: usize, state: &mut SceState) {
        match self.round.pick(i, rand::random::<u8>() as usize & 1) {
            Ok(()) => {
                self.chosen = None;
                self.do_angle = 0.;
            }
            Err(PickError::AlreadyDone) => play_sfx(state, sfx::ALREADY_DONE),
            Err(PickError::NothingLeft) => {}
        }
    }

    /// `FUN_005273d0`。
    fn judge(&mut self, chose_left: bool, state: &mut SceState) {
        let cur = self.round.current;
        if self.round.is_correct(chose_left) {
            state.show_note(self.text(lang::SUCCESS).to_string());
            self.chosen = None;
            self.do_angle = 0.;
            if self.round.mark_correct() {
                play_sfx(state, sfx::ALL_CORRECT);
                self.settle(state);
            }
        } else {
            self.round.mark_wrong();
            play_sfx(state, sfx::WRONG);
            let diffent = self.diffent.get(cur).cloned().unwrap_or_default();
            let text = format!("{}\\n{}", self.text(lang::FAILED), diffent);
            // 原版就是这个顺序：`FUN_005273d0` 末段把 Lang195 送进通用对话框
            // （`FUN_004a1530(1)` 弹出来），同时把这件的 Diffent 写进说明牌的
            // 文字控件 `+0x13f0` 并让它显形，`+0x3abe` 清零、`+0x3abd` 置 1；
            // 等玩家关掉这个框，每帧的 `FUN_00526740` 在 0x526765 轮询到
            // 「关了」，看见 `+0x3abd != 0` 就调 `FUN_00527610` 弹结算框。
            let settle = self.settlement_note();
            self.phase = Phase::Note {
                text: assis_lang::unescape(&text),
                next: Box::new(settle),
            };
        }
    }

    fn settlement_note(&self) -> Phase {
        let fake_names: [String; ITEM_COUNT] = std::array::from_fn(|i| self.names[i].1.clone());
        let text = appraise::result_text(&self.round, &self.lang, &fake_names, self.from_adventure);
        Phase::Note {
            text: assis_lang::unescape(&text),
            next: Box::new(Phase::Finish),
        }
    }

    /// `FUN_00527610`：结算框。
    fn settle(&mut self, _state: &mut SceState) {
        self.phase = self.settlement_note();
    }

    fn draw_do_hud(&self, ui: &Ui, mouse: (f32, f32), drag: Option<Drag>) {
        let round = self.round.clone();
        let queue = self.queue.clone();
        let un_queue = self.un_queue.clone();
        let magnifiers = self.magnifiers.clone();
        let un_magnifier = self.un_magnifier.clone();
        let result = self.result.clone();
        let numbers = self.numbers.clone();
        let time_bar = self.time_bar.clone();
        let magnifier_used = self.magnifier_used;
        let chosen = self.chosen;
        let zoomed = self.do_zoom_side.is_some();
        overlay(ui, "AppraiseDo", |ui, at, _| {
            let list = ui.get_window_draw_list();
            let image = |s: &Option<Rc<Sprite>>, r: Rect, fallback: [f32; 4]| match s {
                Some(s) => list
                    .add_image(s.imgui_texture_id(), at(r.0, r.1), at(r.2, r.3))
                    .build(),
                None => list
                    .add_rect(at(r.0, r.1), at(r.2, r.3), fallback)
                    .filled(true)
                    .build(),
            };

            // 秒数 + 时间条
            let sec = round.timer.sec.clamp(0, 99);
            let digits = [sec / 10, sec % 10];
            // 十位在上、个位在下：初始化里第二个数字控件的矩形是拿第一个的
            // 「下」当「上」再加 0x18 算出来的（0x525a7f 一带）。
            for (k, d) in digits.iter().enumerate() {
                let r = layout::SECONDS;
                let dy = layout::SECONDS_STEP * k as i32;
                let r = (r.0, r.1 + dy, r.2, r.3 + dy);
                image(&numbers[*d as usize], r, HIGHLIGHT);
            }
            let bar = layout::TIME_BAR;
            image(&time_bar[0], bar, PANEL_COLOR);
            let w = round.timer.bar_width();
            let fo = (bar.0, bar.1, bar.0 + w, bar.3);
            match &time_bar[1] {
                // 前景按比例裁一段，免得拉伸。
                Some(s) => list
                    .add_image(s.imgui_texture_id(), at(fo.0, fo.1), at(fo.2, fo.3))
                    .uv_max([w as f32 / 620., 1.])
                    .build(),
                None => list
                    .add_rect(at(fo.0, fo.1), at(fo.2, fo.3), HIGHLIGHT)
                    .filled(true)
                    .build(),
            }

            if zoomed {
                return;
            }

            // 队列
            for i in 0..ITEM_COUNT {
                let r = layout::queue_button(i as i32);
                let sprite = if round.done[i] {
                    &un_queue
                } else if i == round.current {
                    &queue[1]
                } else {
                    &queue[0]
                };
                image(sprite, r, if round.done[i] { PANEL_COLOR } else { WHITE });
                if i == round.current {
                    list.add_rect(at(r.0, r.1), at(r.2, r.3), HIGHLIGHT)
                        .thickness(2.)
                        .build();
                }
            }

            // 放大镜
            for i in 0..3 {
                let r = layout::magnifier(i as i32);
                let dragging = drag.is_some_and(|d| d.magnifier == i);
                if magnifier_used[i] {
                    image(&un_magnifier, r, PANEL_COLOR);
                } else if dragging {
                    let (w, h) = (r.2 - r.0, r.3 - r.1);
                    let m = (mouse.0 as i32 - w / 2, mouse.1 as i32 - h / 2);
                    image(&magnifiers[i], (m.0, m.1, m.0 + w, m.1 + h), WHITE);
                } else {
                    image(&magnifiers[i], r, WHITE);
                }
            }

            // 勾选框 + 确定
            let box_sprite = |on: bool| if on { &result[1] } else { &result[0] };
            image(box_sprite(chosen == Some(true)), layout::RESULT0, WHITE);
            image(box_sprite(chosen == Some(false)), layout::RESULT1, WHITE);
            let c = layout::CONFIRM;
            let hovered = layout::contains(c, mouse.0, mouse.1);
            list.add_rect(
                at(c.0, c.1),
                at(c.2, c.3),
                if hovered { HIGHLIGHT } else { WHITE },
            )
            .thickness(1.)
            .build();
            let _font = game_font(GameFontSize::LARGE).map(|f| ui.push_font(f));
            let size = ui.calc_text_size("确定");
            let (cx, cy) = layout::center(c);
            let p = at(cx as i32, cy as i32);
            list.add_text(
                [p[0] - size[0] * 0.5, p[1] - size[1] * 0.5],
                TEXT_COLOR,
                "确定",
            );
        });
    }

    // ---------- 提示框 ----------

    fn update_note(&mut self, ui: &Ui, text: &str, next: Phase) {
        self.hide_models();
        let text = text.to_string();
        overlay(ui, "AppraiseNote", |ui, at, scale| {
            let list = ui.get_window_draw_list();
            let r = layout::NOTE;
            list.add_rect(at(r.0, r.1), at(r.2, r.3), PANEL_COLOR)
                .filled(true)
                .build();
            list.add_rect(at(r.0, r.1), at(r.2, r.3), HIGHLIGHT)
                .thickness(1.)
                .build();
            let _font = game_font(GameFontSize::SMALL).map(|f| ui.push_font(f));
            let width = (r.2 - r.0 - 16) as f32 * scale;
            let line_h = ui.text_line_height_with_spacing();
            let mut y = at(r.0, r.1)[1] + 8. * scale;
            for para in text.split('\n') {
                let lines = wrap_lines(ui, para, width);
                if lines.is_empty() {
                    y += line_h;
                }
                for line in lines {
                    list.add_text([at(r.0, r.1)[0] + 8. * scale, y], TEXT_COLOR, &line);
                    y += line_h;
                }
            }
        });
        if ui.is_mouse_clicked(MouseButton::Left)
            || ui.is_key_pressed(imgui::Key::Enter)
            || ui.is_key_pressed(imgui::Key::Space)
            || ui.is_key_pressed(imgui::Key::Escape)
        {
            self.phase = next;
        }
    }
}

/// 鼠标在 800×600 虚拟屏幕里的位置。
pub(crate) fn virtual_mouse(ui: &Ui) -> (f32, f32) {
    let [sw, sh] = ui.io().display_size;
    let scale = (sw / layout::SCREEN_W as f32).min(sh / layout::SCREEN_H as f32);
    if scale <= 0. {
        return (-1., -1.);
    }
    let ox = (sw - layout::SCREEN_W as f32 * scale) * 0.5;
    let oy = (sh - layout::SCREEN_H as f32 * scale) * 0.5;
    let [mx, my] = ui.io().mouse_pos;
    ((mx - ox) / scale, (my - oy) / scale)
}

/// 一个铺满屏幕、不吃输入的 imgui 窗口；回调拿到「虚拟坐标 → 屏幕坐标」和
/// 缩放比。
pub(crate) fn overlay<F: FnOnce(&Ui, &dyn Fn(i32, i32) -> [f32; 2], f32)>(
    ui: &Ui,
    name: &str,
    draw: F,
) {
    let [sw, sh] = ui.io().display_size;
    let scale = (sw / layout::SCREEN_W as f32).min(sh / layout::SCREEN_H as f32);
    let ox = (sw - layout::SCREEN_W as f32 * scale) * 0.5;
    let oy = (sh - layout::SCREEN_H as f32 * scale) * 0.5;
    let at = move |x: i32, y: i32| [ox + x as f32 * scale, oy + y as f32 * scale];

    let _pad = ui.push_style_var(imgui::StyleVar::WindowPadding([0., 0.]));
    let _border = ui.push_style_var(imgui::StyleVar::WindowBorderSize(0.));
    ui.window(name)
        .collapsible(false)
        .title_bar(false)
        .resizable(false)
        .movable(false)
        .scroll_bar(false)
        .draw_background(false)
        .no_inputs()
        .position([0., 0.], Condition::Always)
        .size([sw, sh], Condition::Always)
        .build(|| draw(ui, &at, scale));
}

/// 按像素宽度折行（中文没有空格，按字折）。
pub(crate) fn wrap_lines(ui: &Ui, text: &str, width: f32) -> Vec<String> {
    let mut lines = vec![];
    let mut cur = String::new();
    for c in text.chars() {
        let mut trial = cur.clone();
        trial.push(c);
        if !cur.is_empty() && ui.calc_text_size(&trial)[0] > width {
            lines.push(std::mem::take(&mut cur));
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

pub(crate) fn play_sfx(state: &mut SceState, name: &str) {
    let data = state.asset_mgr().load_snd_data(name);
    match data {
        Ok(d) => {
            let mut source = state.audio_engine().create_source();
            source.set_data(d, Codec::Wav);
            source.play(false);
            state
                .global_state_mut()
                .add_sound_source(Rc::new(RefCell::new(source)));
        }
        Err(e) => log::warn!("音效 {name} 读不出来: {e:?}"),
    }
}
