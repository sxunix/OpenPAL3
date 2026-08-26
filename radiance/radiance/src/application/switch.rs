//! Switch application platform.
//!
//! Homebrew has no window system: the applet owns the frame loop and tells us
//! when the user has quit (HOME menu, or the applet being closed). So this is
//! the Vita shape -- a bare loop with a quit flag -- with `appletMainLoop` as
//! the additional exit condition.

use std::os::raw::c_char;

unsafe extern "C" {
    /// Returns false once the applet has been asked to exit.
    fn appletMainLoop() -> bool;
    fn consoleInit(console: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
    fn consoleUpdate(console: *mut core::ffi::c_void);
    fn consoleExit(console: *mut core::ffi::c_void);
    fn svcOutputDebugString(s: *const c_char, len: usize) -> u32;
}

pub struct Platform {
    quit_requested: std::rc::Rc<std::cell::Cell<bool>>,
}

impl Platform {
    pub fn new() -> Self {
        Self {
            quit_requested: std::rc::Rc::new(std::cell::Cell::new(false)),
        }
    }

    pub fn initialize(&mut self) {}

    pub fn run_event_loop<F: FnMut()>(&self, mut update_engine: F) {
        loop {
            // The applet loop is authoritative: once it returns false the
            // system is tearing us down and further GL work is invalid.
            if self.quit_requested.get() || !unsafe { appletMainLoop() } {
                break;
            }
            update_engine();
        }
    }

    pub fn request_exit(&self) {
        self.quit_requested.set(true);
    }

    pub fn set_title(&self, _: &str) {}

    pub fn dpi_scale(&self) -> f32 {
        1.
    }

    pub fn logical_inner_extent(&self) -> Option<(u32, u32)> {
        // Fixed framebuffer, as on Vita -- SceneScaleMode does not apply.
        None
    }

    pub fn show_error_dialog(_title: &str, msg: &str) {
        log::error!("panic: {}", msg);

        // No GUI to put a dialog in, and by this point the GL context is
        // probably unusable. Fall back to the text console plus the debug
        // string service so the message is visible over USB/nxlink too.
        unsafe {
            let bytes = msg.as_bytes();
            svcOutputDebugString(bytes.as_ptr() as *const c_char, bytes.len());

            let console = consoleInit(std::ptr::null_mut());
            // Give the reader a chance to see it before the applet exits.
            for _ in 0..600 {
                if !appletMainLoop() {
                    break;
                }
                consoleUpdate(console);
            }
            consoleExit(console);
        }
    }
}
