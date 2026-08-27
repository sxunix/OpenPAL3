#[cfg(vita)]
mod vita;
#[cfg(switch)]
mod switch;
#[cfg(windows)]
mod windows;
#[cfg(any(linux, macos, android))]
mod winit;

#[cfg(any(linux, macos, android))]
pub use self::winit::ImguiPlatform;
#[cfg(vita)]
pub use vita::ImguiPlatform;
#[cfg(switch)]
pub use switch::ImguiPlatform;
#[cfg(switch)]
pub use switch::LAST_PAD_BUTTONS;
#[cfg(windows)]
pub use windows::ImguiPlatform;
