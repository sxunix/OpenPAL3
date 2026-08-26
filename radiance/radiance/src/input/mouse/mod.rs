#[cfg(any(vita, switch))]
mod nop;
#[cfg(windows)]
mod windows;
#[cfg(any(linux, macos, android))]
mod winit;

#[cfg(any(linux, macos, android))]
pub use self::winit::MouseInput;
#[cfg(any(vita, switch))]
pub use nop::MouseInput;
#[cfg(windows)]
pub use windows::MouseInput;
