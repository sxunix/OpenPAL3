//! 左下角的司南和小地图（`gameui/uiscenemap.cpp` 的 `UISceneMap`，地图本身是
//! `gamebase/scene.cpp` 的 `SceneMapPlayer`）。
//!
//! * 司南：`ui\scene\map\base.tga` 底座（68×23，贴屏幕左下角）、`compass.tga` 罗盘
//!   （61×61，随镜头 yaw 转）、`probe0..2.tga` 指针按钮（64×64，按下展开地图）。
//! * 地图面板：`ui\flex\map` 九宫格，182×182，左下角钉在按钮中心，展开时从左下角
//!   长出来（`UIStaticFlex` 的 openstyle 1，1.15 秒，余弦缓动）；收起是一下就没。
//! * 地图贴图：每层导航网格一张 256×256，`dist`（离边界的格数）为 1、2 的格子画橙色
//!   边、0 画青色（上层只画 `dist > 0`）。室外（SCENE_STORYA）一进来就整张画好，其余
//!   场景（迷宫）只画主角走过的地方——每帧揭开主角周围 33×33 格。
//! * 面板里以主角为中心取地图的一块（宽 0.4、高 0.4/宽高比），中间画 `ui\scene\role.tga`
//!   箭头，按主角朝向转。
//!
//! 只在玩家能操作、不在室内（SCENE_STORYB）时显示（`SceneMgr` / `UISceneFrm`）。
//! [移植] 原版只能用鼠标点按钮；Switch 上加了手柄「-」键（桌面 M 键）开关。
//! [近似] 原版过场过的场景（`Archive::IsScenePast`）进来也整张画好、迷宫探索进度
//! 写进存档；这里两样都没有，迷宫每次进来重新探。

use std::rc::Rc;

use image::{Rgba, RgbaImage};
use imgui::Ui;
use radiance::rendering::Sprite;

use crate::openpal3::{asset_manager::AssetManager, scene::ScnScene};

const IMG: u32 = 256;
/// `SceneMapPlayer::Update`：主角周围 ±16 格。
const FOV: i32 = 16;
/// `Color4ub {B,G,R,A}` 写的 {0,128,255,156} 和 {128,128,0,200}。
const EDGE: Rgba<u8> = Rgba([255, 128, 0, 156]);
const BLOCKED: Rgba<u8> = Rgba([0, 128, 128, 200]);
/// 面板展开时长（`m_mapbk.DURATION`）。
const OPEN_SEC: f32 = 1.15;
/// 迷宫里揭开了新格子时，重新上传地图贴图的最短间隔。
const UPLOAD_EVERY: f32 = 0.25;

/// 一层导航的探索图（`SceneMapPlayer`）。
struct Layer {
    width: i32,
    height: i32,
    /// 上层（`m_nLayerIndex`）只画能走的格子。
    upper: bool,
    explored: Vec<bool>,
    dist: Vec<u8>,
    image: RgbaImage,
    built: bool,
    dirty: bool,
    player: (i32, i32),
    seen_player: bool,
    sprite: Option<Rc<Sprite>>,
}

impl Layer {
    fn new(scene: &ScnScene, layer: usize, upper: bool) -> Self {
        let (w, h) = scene.nav().size(layer);
        let (w, h) = (w as i32, h as i32);
        let mut dist = Vec::with_capacity((w * h) as usize);
        for z in 0..h {
            for x in 0..w {
                dist.push(
                    scene
                        .nav()
                        .get(layer, x, z)
                        .map_or(0, |t| (t.distance_to_border & 0xff) as u8),
                );
            }
        }
        Self {
            width: w,
            height: h,
            upper,
            explored: vec![false; (w * h) as usize],
            dist,
            image: RgbaImage::new(IMG, IMG),
            built: false,
            dirty: true,
            player: (0, 0),
            seen_player: false,
            sprite: None,
        }
    }

    /// `SceneMapPlayer::UpdateRect`：把格子矩形 `[l, r) × [t, b)` 画进图里（`test`
    /// 时只画探过的格子，读档用），不是 `test` 就先记成探过。
    fn update_rect(&mut self, l: i32, t: i32, r: i32, b: i32, test: bool) {
        if !test {
            for z in t.max(0)..=b.min(self.height - 1) {
                for x in l.max(0)..=r.min(self.width - 1) {
                    self.explored[(z * self.width + x) as usize] = true;
                }
            }
        }
        let (w, h) = (self.width as f32, self.height as f32);
        let img = IMG as i32;
        let x0 = ((l as f32 / w) * img as f32) as i32;
        let x1 = ((r as f32 / w) * img as f32) as i32;
        let y0 = ((t as f32 / h) * img as f32) as i32;
        let y1 = ((b as f32 / h) * img as f32) as i32;
        for y in y0.max(0)..y1.min(img - 1) {
            for x in x0.max(0)..x1.min(img - 1) {
                let mx = (x as f32 * w / img as f32) as i32;
                let my = (y as f32 * h / img as f32) as i32;
                let i = (my * self.width + mx) as usize;
                if i >= self.dist.len() || (test && !self.explored[i]) {
                    continue;
                }
                let d = self.dist[i];
                let color = if self.upper {
                    (d > 0).then_some(EDGE)
                } else if d == 1 || d == 2 {
                    Some(EDGE)
                } else if d == 0 {
                    Some(BLOCKED)
                } else {
                    None
                };
                if let Some(c) = color {
                    self.image.put_pixel(x as u32, y as u32, c);
                }
            }
        }
        self.dirty = true;
    }

    fn build(&mut self) {
        self.update_rect(0, 0, self.width, self.height, false);
        self.built = true;
    }
}

#[derive(Default)]
struct Art {
    base: Option<Rc<Sprite>>,
    compass: Option<Rc<Sprite>>,
    probe: Vec<Rc<Sprite>>,
    flex: Vec<Option<Rc<Sprite>>>,
    role: Option<Rc<Sprite>>,
}

#[derive(Default)]
pub struct SceneMapHud {
    art: Option<Art>,
    scene_key: String,
    layers: Vec<Layer>,
    /// 面板开着；`open_t` 是展开动画走了多久。
    open: bool,
    open_t: f32,
    upload_t: f32,
}

/// 这一帧画司南要的场景数据，由调用方从场景和相机里取好。
pub struct SceneMapInput<'a> {
    pub scene: &'a ScnScene,
    pub player_pos: radiance::math::Vec3,
    pub player_facing: radiance::math::Vec3,
    pub player_layer: usize,
    pub camera_yaw: f32,
    pub toggle_key: bool,
}

impl SceneMapHud {
    pub fn render(
        &mut self,
        ui: &Ui,
        delta_sec: f32,
        asset_mgr: &Rc<AssetManager>,
        input: SceneMapInput,
    ) {
        let scene = input.scene;
        // 室内不画、也不探（`SceneMgr::Update` 里 `s_curscene != SCENE_STORYB` 才更新）。
        if scene.scene_type() == 1 {
            return;
        }
        let art = self.art.get_or_insert_with(|| load_art(asset_mgr));

        let key = format!("{}{}", scene.name(), scene.sub_name());
        if key != self.scene_key {
            self.scene_key = key;
            self.open = false;
            let n = scene.nav().layer_count();
            // q08 的三块两层是反着的（`SceneMgr` 换场景那段）。
            let swap = scene.name().eq_ignore_ascii_case("q08")
                && ["q08", "q08f", "q08p"]
                    .iter()
                    .any(|s| scene.sub_name().eq_ignore_ascii_case(s));
            self.layers = (0..n)
                .map(|i| {
                    let index = if swap { (i + 1) % 2 } else { i };
                    let mut l = Layer::new(scene, i, index == 1);
                    if scene.scene_type() == 0 {
                        l.build();
                    }
                    l
                })
                .collect();
        }

        // `SceneMapPlayer::Update`：主角所在格，没整张画好的话揭开周围一块。
        // 站着不动揭不出新格子，换了格子才揭。
        for (i, l) in self.layers.iter_mut().enumerate() {
            let (x, z) = scene.scene_coord_to_nav_coord(i, &input.player_pos);
            let pt = (x as i32, z as i32);
            let moved = pt != l.player || !l.seen_player;
            l.player = pt;
            l.seen_player = true;
            if !l.built && moved {
                l.update_rect(pt.0 - FOV, pt.1 - FOV, pt.0 + FOV, pt.1 + FOV, false);
            }
        }

        let [_, sh] = ui.io().display_size;
        let s = sh / 600.;
        // 原版坐标（800×600，左下角对齐屏幕左下角）换成屏幕坐标。
        let at = |x: f32, y: f32| [x * s, sh - (600. - y) * s];

        let probe_rc = [4., 600. - 5. - 64., 4. + 64., 600. - 5.];
        let [px0, py0] = at(probe_rc[0], probe_rc[1]);
        let [px1, py1] = at(probe_rc[2], probe_rc[3]);
        let mouse = ui.io().mouse_pos;
        let hover = mouse[0] >= px0 && mouse[0] < px1 && mouse[1] >= py0 && mouse[1] < py1;
        let clicked = hover && ui.is_mouse_released(imgui::MouseButton::Left);
        if clicked || input.toggle_key {
            self.open = !self.open;
            self.open_t = 0.;
        }
        if self.open {
            self.open_t = (self.open_t + delta_sec).min(OPEN_SEC);
        }
        let pressed = hover && ui.is_mouse_down(imgui::MouseButton::Left);

        // 地图贴图：开着才传，迷宫里限个频。
        self.upload_t += delta_sec;
        if self.open {
            let due = self.upload_t >= UPLOAD_EVERY;
            for l in &mut self.layers {
                if l.dirty && (l.sprite.is_none() || due) {
                    l.sprite = Some(Rc::new(Sprite::load_from_image(
                        l.image.clone(),
                        asset_mgr.component_factory().as_ref(),
                    )));
                    l.dirty = false;
                }
            }
            if due {
                self.upload_t = 0.;
            }
        }

        let layers = &self.layers;
        let open_t = self.open_t;
        let open = self.open;
        crate::scripting::sce::commands::overlay(ui, "SceneMap", |ui, _at, _scale| {
            let dl = ui.get_window_draw_list();

            if open {
                draw_panel(&dl, art, layers, open_t, input.player_facing, &at, s);
            }

            // 底座：`(0, 600-23)` 起 68×23。
            if let Some(b) = &art.base {
                blit(&dl, b, at(0., 600. - 23.), 68., 23., s);
            }
            // 罗盘：`(4, 600-5-61)` 起 61×61，绕中心转镜头 yaw（度数当角度，逆时针）。
            if let Some(c) = &art.compass {
                let [x, y] = at(4., 600. - 5. - 61.);
                blit_rotated(
                    &dl,
                    c,
                    [x + 30.5 * s, y + 30.5 * s],
                    61.,
                    61.,
                    s,
                    input.camera_yaw,
                    false,
                );
            }
            // 指针按钮：普通 / 悬停 / 按下。
            let state = if pressed {
                2
            } else if hover {
                1
            } else {
                0
            };
            if let Some(p) = art.probe.get(state).or(art.probe.first()) {
                blit(&dl, p, at(probe_rc[0], probe_rc[1]), 64., 64., s);
            }
        });
    }
}

fn load_art(asset_mgr: &Rc<AssetManager>) -> Art {
    let ui = |p: &str| asset_mgr.load_ui_sprite(format!("/basedata/basedata/ui/{p}.tga"));
    Art {
        base: ui("scene/map/base"),
        compass: ui("scene/map/compass"),
        probe: (0..3)
            .filter_map(|i| ui(&format!("scene/map/probe{i}")))
            .collect(),
        flex: (0..9).map(|i| ui(&format!("flex/map{i}"))).collect(),
        role: ui("scene/role"),
    }
}

/// 原版界面图 1:1 从贴图左下角取 `w × h` 画到 `[x, y]`。
fn blit(dl: &imgui::DrawListMut, sp: &Sprite, [x, y]: [f32; 2], w: f32, h: f32, s: f32) {
    let (tw, th) = (sp.width() as f32, sp.height() as f32);
    dl.add_image(sp.imgui_texture_id(), [x, y], [x + w * s, y + h * s])
        .uv_min([0., 1. - h / th])
        .uv_max([w / tw, 1.])
        .build();
}

/// 同 [`blit`]，绕中心 `c` 逆时针转 `deg` 度（原版 `Rotate` 在 y 朝上的屏幕坐标里转）。
/// `whole` 时整张贴图拉满 `w × h`，不按 1:1 裁。
#[allow(clippy::too_many_arguments)]
fn blit_rotated(
    dl: &imgui::DrawListMut,
    sp: &Sprite,
    c: [f32; 2],
    w: f32,
    h: f32,
    s: f32,
    deg: f32,
    whole: bool,
) {
    let (tw, th) = (sp.width() as f32, sp.height() as f32);
    let (sin, cos) = deg.to_radians().sin_cos();
    let (hw, hh) = (w * s * 0.5, h * s * 0.5);
    // 屏幕 y 朝下：视觉上逆时针转 θ 是 (dx, dy) → (dx·cos + dy·sin, −dx·sin + dy·cos)。
    let p = |dx: f32, dy: f32| [c[0] + dx * cos + dy * sin, c[1] - dx * sin + dy * cos];
    let (u1, v0) = if whole {
        (1., 0.)
    } else {
        (w / tw, 1. - h / th)
    };
    dl.add_image_quad(
        sp.imgui_texture_id(),
        p(-hw, -hh),
        p(hw, -hh),
        p(hw, hh),
        p(-hw, hh),
    )
    .uv([0., v0], [u1, v0], [u1, 1.], [0., 1.])
    .build();
}

/// 小地图面板：九宫格外框、各层地图、展开完了再画主角箭头。
fn draw_panel(
    dl: &imgui::DrawListMut,
    art: &Art,
    layers: &[Layer],
    open_t: f32,
    facing: radiance::math::Vec3,
    at: &dyn Fn(f32, f32) -> [f32; 2],
    s: f32,
) {
    // `UISceneMap::Create`：按钮中心 (36, 563) 往右上 182×182。
    let (left, bottom) = (36., 600. - 5. - 32.);
    let (right, top) = (left + 182., bottom - 182.);
    // `UpdateVal`：余弦缓动；openstyle 1 只长右边和上边。
    let x = (open_t / OPEN_SEC).min(1.);
    let e = (1. - (x * std::f32::consts::PI).cos()) * 0.5;
    let cur_r = left + (right - left) * e;
    let cur_t = bottom + (top - bottom) * e;

    // 九宫格（`UIStaticFlex::Render`），角块边长 = min(tile, 半宽, 半高)。
    let tile = art
        .flex
        .first()
        .and_then(|t| t.as_ref())
        .map_or(32., |t| t.width() as f32);
    let cw = tile.min((cur_r - left) / 2.);
    let ch = tile.min((bottom - cur_t) / 2.);
    let rects = [
        (left, cur_t, left + cw, cur_t + ch),
        (left, bottom - ch, left + cw, bottom),
        (cur_r - cw, bottom - ch, cur_r, bottom),
        (cur_r - cw, cur_t, cur_r, cur_t + ch),
        (left, cur_t + ch, left + cw, bottom - ch),
        (left + cw, bottom - ch, cur_r - cw, bottom),
        (cur_r - cw, cur_t + ch, cur_r, bottom - ch),
        (left + cw, cur_t, cur_r - cw, cur_t + ch),
        (left + cw, cur_t + ch, cur_r - cw, bottom - ch),
    ];
    for (i, (x0, y0, x1, y1)) in rects.iter().enumerate() {
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        if let Some(Some(t)) = art.flex.get(i) {
            dl.add_image(t.imgui_texture_id(), at(*x0, *y0), at(*x1, *y1))
                .build();
        }
    }

    // 地图区：外框内缩（左右 5、上 10、下 5）。
    let (ml, mt, mr, mb) = (left + 5., cur_t + 10., cur_r - 5., bottom - 5.);
    if mr <= ml || mb <= mt {
        return;
    }
    // 第 0 号（下层）是比例尺的基准。
    let Some(base) = layers.iter().find(|l| !l.upper).or(layers.first()) else {
        return;
    };
    for l in layers {
        let Some(sp) = &l.sprite else {
            continue;
        };
        let nx = l.player.0 as f32 / l.width.max(1) as f32;
        let ny = l.player.1 as f32 / l.height.max(1) as f32;
        let ratio = l.height as f32 / l.width.max(1) as f32;
        let tw = 0.2 / (l.width as f32 / base.width.max(1) as f32);
        let th = 0.2 / ratio / (l.height as f32 / base.height.max(1) as f32);
        // 取 [nx±tw, ny±th]，出了贴图的部分不画（把屏幕矩形跟着裁掉）。
        let (u0, u1, v0, v1) = (nx - tw, nx + tw, ny - th, ny + th);
        let cu0 = u0.max(0.);
        let cu1 = u1.min(1.);
        let cv0 = v0.max(0.);
        let cv1 = v1.min(1.);
        if cu1 <= cu0 || cv1 <= cv0 {
            continue;
        }
        let sx = |u: f32| ml + (u - u0) / (u1 - u0) * (mr - ml);
        let sy = |v: f32| mt + (v - v0) / (v1 - v0) * (mb - mt);
        let alpha = if l.upper { 200. / 255. } else { 1. };
        dl.add_image(
            sp.imgui_texture_id(),
            at(sx(cu0), sy(cv0)),
            at(sx(cu1), sy(cv1)),
        )
        .uv_min([cu0, cv0])
        .uv_max([cu1, cv1])
        .col([1., 1., 1., alpha])
        .build();
    }

    // 展开完了才画主角（`IsNormalSize`）：面板正中 14×14（quad ±14/ClientWidth），整张
    // 贴图画，按朝向转——
    // `Role` 的 `m_rotate` 是面朝方向和 +z 的夹角，朝 −x 时取 360° 减。
    if x >= 1. {
        if let Some(r) = &art.role {
            let f = radiance::math::Vec3::new(facing.x, 0., facing.z);
            let len = (f.x * f.x + f.z * f.z).sqrt();
            let mut deg = if len > 0. {
                (f.z / len).clamp(-1., 1.).acos().to_degrees()
            } else {
                0.
            };
            if f.x < 0. {
                deg = 360. - deg;
            }
            let c = at((ml + mr) / 2., (mt + mb) / 2.);
            blit_rotated(dl, r, c, 14., 14., s, deg, true);
        }
    }
}
