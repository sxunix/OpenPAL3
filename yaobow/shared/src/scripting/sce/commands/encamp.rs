//! 106 `ENCAMP_Entry` —— 宿营小游戏。规则在 [`crate::openpal3::encamp`]，
//! 逆向笔记在 `switch/re/notes/encamp.md`。
//!
//! 原版 `SwitchMode(10)` 进一个独立模式：营地 pol 场景 + 方位旗 + 五个在队
//! 角色摆成一圈放 `c09`，上面盖一层 UI——说明牌、转盘、四个头像槽、指针、
//! 结果面板。脚本停在 106 上等它结束，没有结果变量，结算直接改精/气/神和
//! 好感。这里做成一条阻塞指令：`initialize` 往场景栈上压一个 [`CoreScene`]，
//! UI 走 imgui。
//!
//! 流程：说明牌 → 点一下 → 空过场框（按键/点击关）→ 转盘起转（`wa033`，
//! 指针每帧减角度，速度按时间加到 5°/帧）→ 点转盘开始减速 → 停下按扇区
//! 得百分比 → 结算（大满贯判定、文字、音效）→ 结果框点「是」→ 每人一块
//! 面板，精/神条 36 px/s 走到当前值 → 精条到头后点转盘退出。
//!
//! 贴图控件按原版画法取 uv：**矩形显示贴图左下角 w×h 那块，正立**
//! （`FUN_005400e0`）。结果框和过场框的贴图（`MUIDialog\Normal\DlgBackground.tga`、
//! `Yes0/1/2.tga`、`UI\Flex\Note`）散文件没有、只在 `UI_opt.tli` 图集里，
//! 这里还是画框代替，位置和尺寸按 `muidialog.cpp` 的算法走。

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use common::store_ext::StoreExt2;
use crosscom::ComRc;
use imgui::{MouseButton, Ui};
use mini_fs::StoreExt;
use radiance::audio::{AudioMemorySource, Codec};
use radiance::comdef::{IEntityExt, ISceneExt, ISceneManager};
use radiance::imgui::{GameFontSize, game_font};
use radiance::math::Vec3;
use radiance::rendering::Sprite;
use radiance::scene::{CoreScene, SceneLighting};

use crate::openpal3::assis_lang;
use crate::openpal3::encamp::{
    self, ASSIS_LANG_PATH, ASSIS_LANG_SECTION, Bar, DIR, Pointer, RMK_PATH, RMK_SECTION,
    ROLE_ACTION, ROLE_MODELS, ROLE_POSITIONS, ROLE_YAWS, RoleStats, SCENE_POL, camera, lang,
    layout::{self, Rect},
    sfx,
};
use crate::openpal3::scene::{RoleAnimationRepeatMode, RoleController};
use crate::scripting::sce::{SceCommand, SceState};

use super::appraise::{
    HIGHLIGHT, PANEL_COLOR, TEXT_COLOR, WHITE, overlay, play_sfx, virtual_mouse, wrap_lines,
};

pub struct SceCommandEncamp {
    site: i32,
    game: Option<Box<EncampGame>>,
    started: bool,
}

impl Clone for SceCommandEncamp {
    fn clone(&self) -> Self {
        Self::new(self.site)
    }
}

impl std::fmt::Debug for SceCommandEncamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SceCommandEncamp")
            .field("site", &self.site)
            .finish()
    }
}

impl SceCommandEncamp {
    pub fn new(site: i32) -> Self {
        Self {
            site,
            game: None,
            started: false,
        }
    }
}

impl SceCommand for SceCommandEncamp {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        if !self.started {
            self.started = true;
            if state.fast_forward() {
                // 跑图：不转盘，按 0% 走一遍结算（等于什么都不恢复），不加好感。
                let present = present_roles(state);
                let lang = read_lang(state);
                let mut stats = read_stats(state);
                encamp::settle(&mut stats, &present, [0, 1, 2, 3], 0., &lang);
                write_stats(state, &stats, &present);
                return true;
            }
            match EncampGame::load(state, self.site) {
                Some(game) => {
                    scene_manager.push_scene(game.scene.clone());
                    self.game = Some(Box::new(game));
                }
                None => log::error!("宿营小游戏的素材读不出来，跳过"),
            }
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
    /// 说明牌，点一下进过场框。
    Remark,
    /// 空的过场框（kind 3），任意键/点击关掉后转盘出现。
    Wait,
    /// 转盘在转；`braking` 是点过转盘之后。
    Spin {
        braking: bool,
    },
    /// 结算文字的对话框（kind 0），点「是」关。
    Result {
        text: String,
    },
    /// 结果面板；`can_exit` 在精条走到头后置位。
    Panels {
        can_exit: bool,
    },
    Done,
}

struct Panel {
    role: usize,
    jing: Bar,
    shen: Bar,
}

struct EncampGame {
    scene: ComRc<radiance::comdef::IScene>,
    /// 第一帧做过进场动作（环境音、头像初值）。
    entered: bool,
    phase: Phase,
    present: [bool; 5],
    lang: Vec<String>,
    remark_text: String,
    pointer: Pointer,
    /// 恢复百分比（`+0x27c8`）。
    pct: f32,
    /// 四个头像槽当前显示谁（`+0x838..`）。
    heads: [usize; 4],
    panels: Vec<Panel>,
    /// 转盘循环音 `wa033`，停下时关掉。
    spin_sound: Option<Rc<RefCell<Box<dyn AudioMemorySource>>>>,

    remark: Option<Rc<Sprite>>,
    turntable: Option<Rc<Sprite>>,
    pointer_tex: Option<Rc<Sprite>>,
    head_tex: [Option<Rc<Sprite>>; 5],
    blance_bk: Option<Rc<Sprite>>,
    blance_head: [Option<Rc<Sprite>>; 5],
    name_frame: [Option<Rc<Sprite>>; 5],
    jing_text: Option<Rc<Sprite>>,
    shen_text: Option<Rc<Sprite>>,
    bar_bk: Option<Rc<Sprite>>,
    jing_fg: Option<Rc<Sprite>>,
    shen_fg: Option<Rc<Sprite>>,
}

fn vfs_path(rel: &str) -> PathBuf {
    PathBuf::from("/basedata/basedata").join(rel)
}

fn present_roles(state: &SceState) -> [bool; 5] {
    let ps = state.global_state().persistent_state();
    std::array::from_fn(|i| ps.is_in_team(i as u32))
}

fn read_lang(state: &SceState) -> Vec<String> {
    state
        .asset_mgr()
        .vfs()
        .read_to_end_from_gbk(vfs_path(ASSIS_LANG_PATH))
        .map(|t: String| assis_lang::parse_section(&t, ASSIS_LANG_SECTION))
        .unwrap_or_default()
}

fn read_stats(state: &SceState) -> [RoleStats; 5] {
    let ps = state.global_state().persistent_state();
    std::array::from_fn(|i| {
        ps.role(i as u32)
            .map(|r| RoleStats {
                hp: r.hp,
                max_hp: r.max_hp,
                sp: r.sp,
                max_sp: r.max_sp,
                mp: r.mp,
                max_mp: r.max_mp,
            })
            .unwrap_or_default()
    })
}

fn write_stats(state: &mut SceState, stats: &[RoleStats; 5], present: &[bool; 5]) {
    let mut ps = state.global_state_mut().persistent_state_mut();
    for (i, (s, on)) in stats.iter().zip(present).enumerate() {
        if !*on {
            continue;
        }
        if let Some(r) = ps.role_mut(i as u32) {
            r.hp = s.hp;
            r.sp = s.sp;
            r.mp = s.mp;
        }
    }
}

impl EncampGame {
    fn load(state: &SceState, site: i32) -> Option<Self> {
        let asset_mgr = state.asset_mgr().clone();
        let vfs = asset_mgr.vfs();
        let factory = asset_mgr.component_factory();

        let lang = read_lang(state);
        let remark_text = vfs
            .read_to_end_from_gbk(vfs_path(RMK_PATH))
            .map(|t: String| assis_lang::parse_section(&t, RMK_SECTION))
            .ok()
            .and_then(|v| v.into_iter().next())
            .map(|s| assis_lang::unescape(&s))
            .unwrap_or_default();
        let present = present_roles(state);

        let scene = CoreScene::create();
        scene.set_lighting(SceneLighting::new([1., 1., 1.], vec![]));

        let scene_pol = vfs_path(SCENE_POL);
        if vfs.open(&scene_pol).is_err() {
            log::error!("营地场景不存在：{}", scene_pol.display());
            return None;
        }
        scene.add_entity(crate::openpal3::loaders::pol::create_entity_from_pol_model(
            &factory,
            vfs,
            &scene_pol,
            "ENCAMP_SCENE".to_string(),
            true,
        ));
        if let Some(flag) = encamp::flag_pol(site) {
            let path = vfs_path(&flag);
            if vfs.open(&path).is_ok() {
                scene.add_entity(crate::openpal3::loaders::pol::create_entity_from_pol_model(
                    &factory,
                    vfs,
                    &path,
                    "ENCAMP_FLAG".to_string(),
                    true,
                ));
            } else {
                log::warn!("方位旗不存在：{}", path.display());
            }
        }

        // 五个在队角色围着营火放 c09。朝向用的是和场景角色同一条路：原版
        // `Encampment_sys.cpp:78` 把 `m_posAngle[i]` 交给 `SetRotObjY`，画的时候
        // `3DObject.cpp:509` 做 `pview->Rotate(m_roty, (0,1,0))`；场景角色那边
        // `role.cpp:302` 也是 `pview->Rotate(m_rotate, y)`，`m_rotate` 由
        // `UpdateRotate`（`role.cpp:264`）从朝向向量算出来：
        // `acos(f·(0,0,1))`，`f.x < 0` 时取 `2π −` 它。两边是同一个角，
        // 所以这里照 [`crate::openpal3::scene::Scene`] 给场景角色那套符号写。
        //
        // 角度本身对得上：五个人的位置重心是 (4.8, −222.6)，按 gbengine 的
        // Y 旋转「模型 +Z 转到 (sin θ, cos θ)」算，256° / 170° / 116° / 80° /
        // 288° 指的方向和各自朝重心的方向差 2°–19°，都是朝着火堆坐的。
        //
        // `ROLE_FACING_BASE = π` 那个基准是全移植端共用的一个校准常量，
        // 不是这里单独定的。
        for i in 0..5 {
            if !present[i] {
                continue;
            }
            let Some(entity) = asset_mgr.load_role(
                ROLE_MODELS[i],
                ROLE_ACTION,
                format!("ENCAMP_ROLE_{i}"),
                true,
            ) else {
                log::warn!("角色模型 {} 读不出来", ROLE_MODELS[i]);
                continue;
            };
            let (x, y, z) = ROLE_POSITIONS[i];
            entity
                .transform()
                .borrow_mut()
                .set_position(&Vec3::new(x, y, z))
                .rotate_axis_angle_local(
                    &Vec3::UP,
                    std::f32::consts::PI - ROLE_YAWS[i].to_radians(),
                );
            if let Some(rc) = RoleController::get_role_controller(entity.clone()) {
                let rc = rc.inner::<RoleController>();
                rc.set_active(true);
                rc.play_anim(ROLE_ACTION, RoleAnimationRepeatMode::Loop);
            }
            scene.add_entity(entity);
        }

        {
            let mut cam = scene.camera_mut();
            cam.set_fov43(camera::FOV_DEG.to_radians());
            let (ex, ey, ez) = camera::eye();
            let (tx, ty, tz) = camera::TARGET;
            cam.transform_mut()
                .set_position(&Vec3::new(ex, ey, ez))
                .look_at(&Vec3::new(tx, ty, tz));
        }

        let sprite = |name: &str| asset_mgr.load_sprite(vfs_path(name));
        let sub = |name: &str| sprite(&format!("{DIR}/{name}"));

        Some(Self {
            scene,
            entered: false,
            phase: Phase::Remark,
            present,
            lang,
            remark_text,
            pointer: Pointer::new(),
            pct: 0.,
            heads: [0; 4],
            panels: vec![],
            spin_sound: None,
            remark: sub("Remark.tga"),
            turntable: sub("TurnTable.tga"),
            pointer_tex: sub("Pointer.tga"),
            head_tex: std::array::from_fn(|i| sub(&format!("head{i}.tga"))),
            blance_bk: sub("BlanceBk.tga"),
            blance_head: std::array::from_fn(|i| sub(&format!("BlanceHead{i}.tga"))),
            name_frame: std::array::from_fn(|i| {
                sprite(&format!("UI/GameMainUI/Communal/AttFrameHead{i}.tga"))
            }),
            jing_text: sub("JingText.tga"),
            shen_text: sub("ShenText.tga"),
            bar_bk: sub("BarBk.tga"),
            jing_fg: sub("JingProgress.tga"),
            shen_fg: sub("ShenProgress.tga"),
        })
    }

    fn text(&self, i: usize) -> &str {
        self.lang.get(i).map(String::as_str).unwrap_or("")
    }

    fn name(&self, k: usize) -> &str {
        self.text(lang::NAMES + k)
    }

    /// `FUN_0052d5b0`：四个槽各抽一个在队的人。
    fn shuffle_heads(&mut self) {
        if !self.present.iter().any(|p| *p) {
            return;
        }
        for h in self.heads.iter_mut() {
            loop {
                let k = rand::random::<u32>() as usize % 5;
                if self.present[k] {
                    *h = k;
                    break;
                }
            }
        }
    }

    fn update(&mut self, ui: &Ui, state: &mut SceState, delta_sec: f32) {
        if !self.entered {
            self.entered = true;
            // 进场：`wa032` 环境循环（借 spin_sound 槽放，起转时换成 wa033），
            // 四个头像槽先各抽一个人。
            self.start_loop(state, sfx::AMBIENT);
            self.shuffle_heads();
        }
        let phase = self.phase.clone();
        match phase {
            Phase::Remark => self.update_remark(ui, state),
            Phase::Wait => self.update_wait(ui, state),
            Phase::Spin { braking } => self.update_spin(ui, state, delta_sec, braking),
            Phase::Result { text } => self.update_result(ui, state, &text),
            Phase::Panels { can_exit } => self.update_panels(ui, state, delta_sec, can_exit),
            Phase::Done => {}
        }
    }

    fn leave(&mut self, state: &mut SceState) {
        self.stop_spin_sound();
        state.global_state_mut().play_default_bgm();
    }

    fn start_loop(&mut self, state: &mut SceState, name: &str) {
        match state.asset_mgr().load_snd_data(name) {
            Ok(d) => {
                let mut source = state.audio_engine().create_source();
                source.set_data(d, Codec::Wav);
                source.play(true);
                let source = Rc::new(RefCell::new(source));
                state.global_state_mut().add_sound_source(source.clone());
                self.spin_sound = Some(source);
            }
            Err(e) => log::warn!("音效 {name} 读不出来: {e:?}"),
        }
    }

    fn stop_spin_sound(&mut self) {
        if let Some(s) = self.spin_sound.take() {
            s.borrow_mut().stop();
        }
    }

    // ---------- 说明牌 ----------

    fn update_remark(&mut self, ui: &Ui, _state: &mut SceState) {
        let text = self.remark_text.clone();
        let remark = self.remark.clone();
        overlay(ui, "EncampRemark", |ui, at, scale| {
            let list = ui.get_window_draw_list();
            draw_sprite(&list, remark.as_deref(), layout::REMARK, at);
            let _font = game_font(GameFontSize::LARGE).map(|f| ui.push_font(f));
            let r = layout::REMARK_TEXT;
            let width = (r.2 - r.0) as f32 * scale;
            let line_h = ui.text_line_height_with_spacing();
            let mut y = at(r.0, r.1)[1];
            for para in text.split('\n') {
                let lines = wrap_lines(ui, para, width);
                if lines.is_empty() {
                    y += line_h;
                }
                for line in lines {
                    list.add_text([at(r.0, r.1)[0], y], TEXT_COLOR, &line);
                    y += line_h;
                }
            }
        });
        // 原版是纯鼠标：点在说明牌上才往下走。[移植] 同时收任意键——Switch 上只有
        // 触屏会变成 imgui 鼠标（`imgui/platform/switch.rs`），插底座只用手柄时
        // 没有指针，这一屏就再也过不去。和下一步过场框同一个 `any_key`。
        let mouse = virtual_mouse(ui);
        if (ui.is_mouse_clicked(MouseButton::Left)
            && layout::contains(layout::REMARK, mouse.0, mouse.1))
            || any_key(ui)
        {
            self.phase = Phase::Wait;
        }
    }

    // ---------- 过场框（kind 3） ----------

    fn update_wait(&mut self, ui: &Ui, state: &mut SceState) {
        overlay(ui, "EncampWait", |ui, at, _| {
            let list = ui.get_window_draw_list();
            draw_frame(&list, layout::WAIT_DIALOG, at);
        });
        if ui.is_mouse_clicked(MouseButton::Left) || any_key(ui) {
            // 转盘和指针出现，起转，`wa033` 循环
            self.stop_spin_sound();
            self.start_loop(state, sfx::SPIN);
            self.pointer = Pointer::new();
            self.phase = Phase::Spin { braking: false };
        }
    }

    // ---------- 转盘 ----------

    fn update_spin(&mut self, ui: &Ui, state: &mut SceState, delta_sec: f32, braking: bool) {
        if braking {
            self.pointer.start_braking();
        }
        let stopped = self.pointer.step(delta_sec);
        self.shuffle_heads();
        self.draw_table(ui, true);

        if stopped {
            self.pct = encamp::sector_percent(self.pointer.angle, self.pct);
            self.stop_spin_sound();
            self.shuffle_heads();
            self.settle(state);
            return;
        }
        // 点转盘开始减速。[移植] 同上，手柄按确定也算。
        let mouse = virtual_mouse(ui);
        let clicked = ui.is_mouse_clicked(MouseButton::Left)
            && layout::contains(layout::TURNTABLE, mouse.0, mouse.1);
        if !braking && (clicked || any_key(ui)) {
            self.phase = Phase::Spin { braking: true };
        }
    }

    /// `FUN_0052d230`。
    fn settle(&mut self, state: &mut SceState) {
        let mut stats = read_stats(state);
        let r = encamp::settle(&mut stats, &self.present, self.heads, self.pct, &self.lang);
        write_stats(state, &stats, &self.present);
        if let Some(k) = r.favor_plus_ten {
            state
                .global_state_mut()
                .persistent_state_mut()
                .add_favor(k as u32, 10);
        }
        if let Some((_, s)) = r.jackpot {
            play_sfx(state, s);
        }
        // 原版循环五次每次放一声 wd001，叠在一起；这里放一声。
        play_sfx(state, sfx::RECOVER);
        self.phase = Phase::Result { text: r.text };
    }

    // ---------- 结果框（kind 0） ----------

    fn update_result(&mut self, ui: &Ui, state: &mut SceState, text: &str) {
        self.draw_table(ui, false);
        let text = text.to_string();
        let mouse = virtual_mouse(ui);
        let ok = layout::RESULT_YES;
        overlay(ui, "EncampResult", |ui, at, scale| {
            let list = ui.get_window_draw_list();
            let r = layout::RESULT_DIALOG;
            draw_frame(&list, r, at);
            let _font = game_font(GameFontSize::SMALL).map(|f| ui.push_font(f));
            let line_h = ui.text_line_height_with_spacing();
            let mut y = at(r.0, r.1)[1] + 6. * scale;
            for line in text.split('\n') {
                list.add_text([at(r.0, r.1)[0] + 6. * scale, y], TEXT_COLOR, line);
                y += line_h;
            }
            let hovered = layout::contains(ok, mouse.0, mouse.1);
            list.add_rect(
                at(ok.0, ok.1),
                at(ok.2, ok.3),
                if hovered { HIGHLIGHT } else { WHITE },
            )
            .thickness(1.)
            .build();
            let label = "是";
            let size = ui.calc_text_size(label);
            let (cx, cy) = layout::center(ok);
            let p = at(cx as i32, cy as i32);
            list.add_text(
                [p[0] - size[0] * 0.5, p[1] - size[1] * 0.5],
                TEXT_COLOR,
                label,
            );
        });
        let clicked =
            ui.is_mouse_clicked(MouseButton::Left) && layout::contains(ok, mouse.0, mouse.1);
        // [移植] `any_key` 里有手柄 A / B——Switch 的手柄只产生 imgui 的
        // `Gamepad*` 键，不产生回车；插底座没有触屏时只认回车就卡在这里。
        if clicked || any_key(ui) {
            play_sfx(state, sfx::OK);
            self.show_panels(state);
        }
    }

    /// `FUN_0052d0f0`：在队角色各一块面板，条从 0 走到当前值。
    fn show_panels(&mut self, state: &mut SceState) {
        let stats = read_stats(state);
        self.panels = (0..5)
            .filter(|i| self.present[*i])
            .map(|i| {
                let s = &stats[i];
                let mut jing = Bar::new(s.max_hp);
                let mut shen = Bar::new(s.max_mp);
                jing.reset();
                shen.reset();
                jing.set_target(s.hp);
                shen.set_target(s.mp);
                Panel {
                    role: i,
                    jing,
                    shen,
                }
            })
            .collect();
        self.phase = Phase::Panels { can_exit: false };
    }

    // ---------- 结果面板 ----------

    fn update_panels(&mut self, ui: &Ui, _state: &mut SceState, delta_sec: f32, can_exit: bool) {
        for p in self.panels.iter_mut() {
            p.jing.step(delta_sec);
            p.shen.step(delta_sec);
        }
        self.draw_table(ui, false);

        let mouse = virtual_mouse(ui);
        overlay(ui, "EncampPanels", |ui, at, _| {
            let list = ui.get_window_draw_list();
            let _font = game_font(GameFontSize::SMALL).map(|f| ui.push_font(f));
            for (n, p) in self.panels.iter().enumerate() {
                let Some(&x) = layout::PANEL_X.get(n) else {
                    break;
                };
                let k = p.role;
                let base = |r: Rect| layout::offset(r, x, 0);
                draw_sprite(
                    &list,
                    self.blance_bk.as_deref(),
                    (x, 0, x + layout::PANEL_W, layout::PANEL_H),
                    at,
                );
                let nf = base(layout::PANEL_NAME_FRAME);
                if self.name_frame[k].is_some() {
                    draw_sprite(&list, self.name_frame[k].as_deref(), nf, at);
                } else {
                    // 名字框贴图不在包里：画框写名字。
                    list.add_rect(at(nf.0, nf.1), at(nf.2, nf.3), HIGHLIGHT)
                        .thickness(1.)
                        .build();
                    let name = self.name(k);
                    let size = ui.calc_text_size(name);
                    let (cx, cy) = layout::center(nf);
                    let c = at(cx as i32, cy as i32);
                    list.add_text(
                        [c[0] - size[0] * 0.5, c[1] - size[1] * 0.5],
                        TEXT_COLOR,
                        name,
                    );
                }
                draw_sprite(
                    &list,
                    self.blance_head[k].as_deref(),
                    base(layout::PANEL_HEAD),
                    at,
                );
                draw_sprite(
                    &list,
                    self.jing_text.as_deref(),
                    base(layout::PANEL_JING_TEXT),
                    at,
                );
                draw_sprite(
                    &list,
                    self.shen_text.as_deref(),
                    base(layout::PANEL_SHEN_TEXT),
                    at,
                );
                draw_bar(
                    &list,
                    self.bar_bk.as_deref(),
                    self.jing_fg.as_deref(),
                    base(layout::PANEL_JING_BAR),
                    p.jing.fill_px(),
                    at,
                );
                draw_bar(
                    &list,
                    self.bar_bk.as_deref(),
                    self.shen_fg.as_deref(),
                    base(layout::PANEL_SHEN_BAR),
                    p.shen.fill_px(),
                    at,
                );
                let jv = base(layout::PANEL_JING_VALUE);
                let sv = base(layout::PANEL_SHEN_VALUE);
                list.add_text(
                    at(jv.0, jv.1),
                    TEXT_COLOR,
                    format!("{}/{}", p.jing.display_value(), p.jing.max),
                );
                list.add_text(
                    at(sv.0, sv.1),
                    TEXT_COLOR,
                    format!("{}/{}", p.shen.display_value(), p.shen.max),
                );
            }
        });

        // 精条走到头（`+0xa0`）之后才接受退出。
        let arrived = self.panels.is_empty() || self.panels.iter().all(|p| p.jing.arrived);
        if !can_exit {
            if arrived {
                self.phase = Phase::Panels { can_exit: true };
            }
            return;
        }
        let clicked = ui.is_mouse_clicked(MouseButton::Left)
            && layout::contains(layout::TURNTABLE, mouse.0, mouse.1);
        // [移植] 同上，补手柄（`any_key` 已含回车 / Esc / A / B）。
        if clicked || any_key(ui) {
            self.phase = Phase::Done;
        }
    }

    /// 转盘 + 四个头像 + 指针。
    fn draw_table(&self, ui: &Ui, _spinning: bool) {
        let angle = self.pointer.angle;
        overlay(ui, "EncampTable", |ui, at, _| {
            let list = ui.get_window_draw_list();
            draw_sprite(&list, self.turntable.as_deref(), layout::TURNTABLE, at);
            for (slot, r) in layout::HEADS.iter().enumerate() {
                let k = self.heads[slot];
                draw_sprite(&list, self.head_tex[k].as_deref(), *r, at);
            }
            draw_rotated_sprite(
                &list,
                self.pointer_tex.as_deref(),
                layout::POINTER,
                angle,
                at,
            );
        });
    }
}

fn any_key(ui: &Ui) -> bool {
    [
        imgui::Key::Enter,
        imgui::Key::Space,
        imgui::Key::Escape,
        imgui::Key::GamepadFaceDown,
        imgui::Key::GamepadFaceRight,
    ]
    .iter()
    .any(|k| ui.is_key_pressed(*k))
}

/// 原版贴图控件的 uv：矩形显示贴图**左下角** w×h 那块，正立（`FUN_005400e0`
/// + gbengine 对左下原点 TGA 不翻转）。`image` 解出来的图是正立的，所以
/// uv 从 (0, 1 − h/H) 到 (w/W, 1)。
pub(crate) fn sprite_uv(s: &Sprite, r: Rect) -> ([f32; 2], [f32; 2]) {
    let (tw, th) = (s.width().max(1) as f32, s.height().max(1) as f32);
    let (w, h) = ((r.2 - r.0) as f32, (r.3 - r.1) as f32);
    ([0., (1. - h / th).max(0.)], [(w / tw).min(1.), 1.])
}

pub(crate) fn draw_sprite(
    list: &imgui::DrawListMut<'_>,
    sprite: Option<&Sprite>,
    r: Rect,
    at: &dyn Fn(i32, i32) -> [f32; 2],
) {
    draw_sprite_col(list, sprite, r, [1., 1., 1., 1.], at);
}

/// 同 [`draw_sprite`]，贴图乘一个颜色。
pub(crate) fn draw_sprite_col(
    list: &imgui::DrawListMut<'_>,
    sprite: Option<&Sprite>,
    r: Rect,
    col: [f32; 4],
    at: &dyn Fn(i32, i32) -> [f32; 2],
) {
    let Some(s) = sprite else { return };
    let (uv0, uv1) = sprite_uv(s, r);
    list.add_image(s.imgui_texture_id(), at(r.0, r.1), at(r.2, r.3))
        .uv_min(uv0)
        .uv_max(uv1)
        .col(col)
        .build();
}

/// 绕矩形中心转 `angle` 度（`FUN_00442e90`）。
///
/// **正角是逆时针**，已查实：`FUN_00442e90` 把控件中心换成 NDC 后压平移，
/// 再用 `gbMatrixStack::Rotate(角度, {0,0,1})` 绕 +Z 转
/// （gbengine.dll `0x100271e0`：四元数 `w=cos(度·π/360)`、`xyz=sin(…)·轴`，
/// `gbQuaternion::GetMatrix` 0x10026b60 填出 `m00=1−yy−zz`、`m01=xy+wz`、
/// `m10=xy−wz`，绕 Z 就是 `[[cosθ, sinθ],[−sinθ, cosθ]]`）。
/// 引擎是行向量（`gbMatrix4::Transform` 0x100252c0 算的是
/// `out.x = x·m00 + y·m10 + z·m20 + m30`），所以 `(1,0,0)` 转到
/// `(cosθ, sinθ)`——在 +Y 朝上的 NDC 里就是逆时针。
/// 指针的角度是往下减的，所以画面上看是顺时针转。
fn draw_rotated_sprite(
    list: &imgui::DrawListMut<'_>,
    sprite: Option<&Sprite>,
    r: Rect,
    angle_deg: f32,
    at: &dyn Fn(i32, i32) -> [f32; 2],
) {
    let Some(s) = sprite else { return };
    let (uv0, uv1) = sprite_uv(s, r);
    let (cx, cy) = layout::center(r);
    let c = at(cx as i32, cy as i32);
    let p0 = at(r.0, r.1);
    let p1 = at(r.2, r.3);
    let (sin, cos) = angle_deg.to_radians().sin_cos();
    let rot = |x: f32, y: f32| {
        let (dx, dy) = (x - c[0], y - c[1]);
        [c[0] + dx * cos + dy * sin, c[1] - dx * sin + dy * cos]
    };
    list.add_image_quad(
        s.imgui_texture_id(),
        rot(p0[0], p0[1]),
        rot(p1[0], p0[1]),
        rot(p1[0], p1[1]),
        rot(p0[0], p1[1]),
    )
    .uv(uv0, [uv1[0], uv0[1]], uv1, [uv0[0], uv1[1]])
    .build();
}

/// 进度条：底图铺满矩形，前景填 `(左 + 2, 上 + 3)` 到 `(左 + 2 + fill, 下 − 3)`，
/// 整张前景图拉伸进这块，不是裁出来的。
///
/// 控件是 `UIProgressBar`（`ui/uiprogressbar.cpp`，exe 里那四个边距 setter
/// `FUN_0043fd90`/`fdc0`/`fdd0`/`fde0` 分别是 `SetRadiusWidth`/`SetLRadius`/
/// `SetRRadius`/`SetTRadius`…，四角在 `0x43ffe0` 一带算）。横竖两向的公式和
/// 宿营这两条用的边距值都写在 [`crate::openpal3::encamp::layout::BAR_INSET_L`]
/// 上面。
///
/// 当时没分辨出来的 `+0xc4`/`+0xc8` 是 `m_decorateW`/`m_decorateH`——填充条
/// 末端那个固定尺寸的小装饰件，在轨道里竖向居中。宿营两条没设它
/// （`m_pdecorate == NULL`），所以不画。
///
/// 拉伸这件事来自 `Render` 里这一支：`m_dir == LEFT && m_ppicInfo != NULL`
/// 时走 `UIRenderQuad(m_ProgressRc, …, m_ppicInfo, false)`，`on == false`
/// 让四边取 `m_ProgressRc`、uv 仍取图集里这张图的整块，于是整张图被挤进
/// 填充矩形。宿营的 `m_opt` 是 `true`（`Encampment_ui.cpp:18`），贴图都走
/// `UI_opt.tli` 图集，所以 `m_ppicInfo` 非空，走的就是这一支。
fn draw_bar(
    list: &imgui::DrawListMut<'_>,
    bk: Option<&Sprite>,
    fg: Option<&Sprite>,
    r: Rect,
    fill_px: f32,
    at: &dyn Fn(i32, i32) -> [f32; 2],
) {
    draw_sprite(list, bk, r, at);
    let Some(fg) = fg else { return };
    let fill = fill_px.round() as i32;
    if fill <= 0 {
        return;
    }
    let fr = (
        r.0 + layout::BAR_INSET_L,
        r.1 + layout::BAR_INSET_T,
        r.0 + layout::BAR_INSET_L + fill,
        r.3 - layout::BAR_INSET_B,
    );
    list.add_image(fg.imgui_texture_id(), at(fr.0, fr.1), at(fr.2, fr.3))
        .uv_min([0., 0.])
        .uv_max([1., 1.])
        .build();
}

fn draw_frame(list: &imgui::DrawListMut<'_>, r: Rect, at: &dyn Fn(i32, i32) -> [f32; 2]) {
    list.add_rect(at(r.0, r.1), at(r.2, r.3), PANEL_COLOR)
        .filled(true)
        .build();
    list.add_rect(at(r.0, r.1), at(r.2, r.3), HIGHLIGHT)
        .thickness(1.)
        .build();
}
