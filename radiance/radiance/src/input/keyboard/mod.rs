#[cfg(any(vita, switch))]
mod nop;
#[cfg(windows)]
mod windows;
#[cfg(any(linux, macos, android))]
mod winit;

#[cfg(any(linux, macos, android))]
pub use self::winit::KeyboardInput;
#[cfg(any(vita, switch))]
pub use nop::KeyboardInput;
#[cfg(windows)]
pub use windows::KeyboardInput;
