use std::{cell::RefCell, rc::Rc};

use crosscom::ComRc;
use imgui::{Condition, Image, TextureId, Ui};
use radiance::{
    comdef::{IScene, ISceneExt},
    input::{Axis, InputEngine, Key},
    math::{Mat44, Vec3},
    rendering::VideoPlayer,
};

pub fn show_video_window(
    ui: &Ui,
    video_player: &mut VideoPlayer,
    texture_id: Option<TextureId>,
    window_size: [f32; 2],
    target_size: [f32; 2],
) -> Option<TextureId> {
    let mut ret_texture_id = None;
    ui.window("video")
        .size(window_size, Condition::Always)
        .position([0.0, 0.0], Condition::Always)
        .always_auto_resize(false)
        .draw_background(false)
        .scrollable(false)
        .no_decoration()
        .movable(false)
        .build(|| {
            if let Some(texture_id) = video_player.get_texture(texture_id) {
                ui.set_cursor_pos([
                    (window_size[0] - target_size[0]) * 0.5,
                    (window_size[1] - target_size[1]) * 0.5,
                ]);
                Image::new(texture_id, target_size).build(ui);
                ret_texture_id = Some(texture_id)
            }
        });

    ret_texture_id
}

pub fn play_movie(
    ui: &Ui,
    video_player: &mut VideoPlayer,
    texture_id: Option<TextureId>,
    source_size: (u32, u32),
    remove_black_bars: bool,
) -> Option<TextureId> {
    let window_size = ui.io().display_size;
    let (source_w, source_h) = source_size;

    // Keep aspect ratio
    let w_scale = window_size[0] / source_w as f32;
    let h_scale = if remove_black_bars {
        // Some of PAL3 movies are 4:3 ones with black bars on top and bottom
        // Scale movies to remove the black bars
        let new_source_h = source_w * 9 / 16;
        window_size[1] / new_source_h as f32
    } else {
        window_size[1] / source_h as f32
    };

    let scale = w_scale.min(h_scale);
    let target_size = [source_w as f32 * scale, source_h as f32 * scale];

    show_video_window(ui, video_player, texture_id, window_size, target_size)
}

/// 方向（世界系，已归一化）加上**推力**——摇杆推了多满，键盘按下就是 1。
///
/// 原版靠推力大小分走和跑，所以归一化之前得先把长度留下来。
pub fn get_moving_direction_and_throw(
    input: Rc<RefCell<dyn InputEngine>>,
    scene: ComRc<IScene>,
) -> (Vec3, f32) {
    let input = input.borrow_mut();
    let mut local_direction = Vec3::new(0., 0., 0.);

    if input.get_key_state(Key::Up).is_down() || input.get_key_state(Key::GamePadDPadUp).is_down() {
        local_direction = Vec3::add(&local_direction, &Vec3::new(0., 0., -1.));
    }

    if input.get_key_state(Key::Down).is_down()
        || input.get_key_state(Key::GamePadDPadDown).is_down()
    {
        local_direction = Vec3::add(&local_direction, &Vec3::new(0., 0., 1.));
    }

    let left_y = input.get_axis_state(Axis::LeftStickY).value();
    let left_y = if left_y.abs() < 0.1 { 0. } else { left_y };
    local_direction = Vec3::add(&local_direction, &Vec3::new(0., 0., -left_y));

    if input.get_key_state(Key::Left).is_down()
        || input.get_key_state(Key::GamePadDPadLeft).is_down()
    {
        local_direction = Vec3::add(&local_direction, &Vec3::new(-1., 0., 0.));
    }

    if input.get_key_state(Key::Right).is_down()
        || input.get_key_state(Key::GamePadDPadRight).is_down()
    {
        local_direction = Vec3::add(&local_direction, &Vec3::new(1., 0., 0.));
    }

    let left_x = input.get_axis_state(Axis::LeftStickX).value();
    let left_x = if left_x.abs() < 0.1 { 0. } else { left_x };

    local_direction = Vec3::add(&local_direction, &Vec3::new(left_x, 0., 0.));
    // 归一化会把摇杆推了多满这件事抹掉，先留一份。键盘同时按两个方向
    // 会到 1.41，夹到 1。
    let throw = local_direction.norm().min(1.);
    local_direction.normalize();

    let camera_mat = scene.camera().transform().matrix().clone();

    let mut world_direction_mat = Mat44::new_zero();
    world_direction_mat[0][3] = local_direction.x;
    world_direction_mat[1][3] = local_direction.y;
    world_direction_mat[2][3] = local_direction.z;
    let world_direction_mat = Mat44::multiplied(&camera_mat, &world_direction_mat);

    let world_direction = Vec3::new(world_direction_mat[0][3], 0., world_direction_mat[2][3]);
    (Vec3::normalized(&world_direction), throw)
}

/// 只要方向的老接口。
pub fn get_moving_direction(input: Rc<RefCell<dyn InputEngine>>, scene: ComRc<IScene>) -> Vec3 {
    get_moving_direction_and_throw(input, scene).0
}

pub fn get_camera_rotation(
    input: Rc<RefCell<dyn InputEngine>>,
    mut current_rotation: f32,
    delta_sec: f32,
) -> f32 {
    let input = input.borrow();
    const CAMERA_ROTATE_SPEED: f32 = 1.5;

    if input.get_key_state(Key::A).is_down() {
        current_rotation -= CAMERA_ROTATE_SPEED * delta_sec;
    }

    if input.get_key_state(Key::D).is_down() {
        current_rotation += CAMERA_ROTATE_SPEED * delta_sec;
    }

    let right_x = input.get_axis_state(Axis::RightStickX).value();
    let right_x = if right_x.abs() < 0.1 { 0. } else { right_x };

    current_rotation -= CAMERA_ROTATE_SPEED * delta_sec * right_x;

    // Mouse wheel matches the original PC binding (滚轮 = 转换视觉).
    // `get_mouse_wheel` reports detents accumulated over the last engine
    // update, so this steps per tick rather than per frame.
    const WHEEL_ROTATE_STEP: f32 = 0.12;
    current_rotation -= input.get_mouse_wheel() * WHEEL_ROTATE_STEP;

    if current_rotation < 0. {
        current_rotation += std::f32::consts::PI * 2.;
    }

    if current_rotation > std::f32::consts::PI * 2. {
        current_rotation -= std::f32::consts::PI * 2.;
    }

    current_rotation
}
