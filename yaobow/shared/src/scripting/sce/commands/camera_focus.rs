//! What the camera keeps in frame, and its state stack.
//!
//! These five all move within the polar camera model in
//! [`Pal3Camera`](crate::openpal3::states::camera::Pal3Camera); none of
//! them blocks the script, and none of them touches the angles.

use crate::openpal3::states::camera::Focus;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;
use radiance::math::Vec3;

macro_rules! camera_command {
    ($name:ident, $doc:expr, $body:expr $(, $field:ident : $ty:ty)* $(,)?) => {
        #[doc = $doc]
        #[derive(Debug, Clone)]
        pub struct $name {
            $($field: $ty,)*
        }

        impl $name {
            pub fn new($($field: $ty),*) -> Self {
                Self { $($field),* }
            }
        }

        impl SceCommand for $name {
            fn update(
                &mut self,
                _scene_manager: ComRc<ISceneManager>,
                _ui: &Ui,
                state: &mut SceState,
                _delta_sec: f32,
            ) -> bool {
                #[allow(clippy::redundant_closure_call)]
                ($body)(self, state.global_state_mut().camera_mut());
                true
            }
        }
    };
}

camera_command!(
    SceCommandCameraFocusRole,
    "30 摄像机锁定角色 — `LAB_00435320` → `FUN_0041c170`，把角色存进 \
     `+0x17c` 并清掉 `+0x180`。持续锁定，不是一次性看一眼。",
    |me: &Self, camera: &mut crate::openpal3::states::camera::Pal3Camera| {
        camera.set_focus(Focus::Role(me.role_id));
    },
    role_id: i32
);

camera_command!(
    SceCommandCameraFocusObject,
    "251 摄像机锁定场景物品 — `LAB_00435350` → `FUN_0041c190`，写 \
     `+0x180` 并清掉 `+0x17c`。与 30 互斥。",
    |me: &Self, camera: &mut crate::openpal3::states::camera::Pal3Camera| {
        camera.set_focus(Focus::Object(me.object_id));
    },
    object_id: i32
);

camera_command!(
    SceCommandCameraFocusPoint,
    "31 摄像机锁定定点 — `LAB_004352d0` → `FUN_0041c1b0`，两个目标指针 \
     都清零，坐标存 `+0x184`。GameBox 世界坐标。",
    |me: &Self, camera: &mut crate::openpal3::states::camera::Pal3Camera| {
        camera.set_focus(Focus::Point(Vec3::new(me.x, me.y, me.z)));
    },
    x: f32,
    y: f32,
    z: f32
);

camera_command!(
    SceCommandCameraFollowPlayer,
    "250 镜头是否跟随主角 — `LAB_00435380` 只写 `+0x1c0`。注意它**不**清 \
     锁定目标；Pal3.Unity 的实现顺手清了，这里按二进制。",
    |me: &Self, camera: &mut crate::openpal3::states::camera::Pal3Camera| {
        camera.set_follow_player(me.follow != 0);
        if me.follow != 0 {
            camera.set_focus(Focus::Player);
        }
    },
    follow: i32
);

camera_command!(
    SceCommandCameraPushState,
    "38 保存镜头状态 — `FUN_0041d220` 压入 yaw、pitch、距离和相机位置。",
    |_me: &Self, camera: &mut crate::openpal3::states::camera::Pal3Camera| {
        camera.push_state();
    },
);

camera_command!(
    SceCommandCameraPopState,
    "39 取出镜头状态 — `FUN_0041d410` 恢复那四项，然后按相机位置固定 \
     重算 lookAt（`FUN_0041c710(1)`）。锁定目标不在栈里，弹出后仍然生效。",
    |_me: &Self, camera: &mut crate::openpal3::states::camera::Pal3Camera| {
        if !camera.pop_state() {
            log::warn!("CameraPopState: 状态栈是空的");
        }
    },
);
