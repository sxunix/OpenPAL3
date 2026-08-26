#[cfg(any(vita, switch))]
mod dummy;
#[cfg(windows)]
mod windows;

#[cfg(any(linux, macos, android))]
pub use ::winit::window::Window;
#[cfg(windows)]
pub use windows::Window;

#[cfg(any(vita, switch))]
pub use dummy::Window;
