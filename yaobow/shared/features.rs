use cfg_aliases::cfg_aliases;

pub fn enable_features() {
    cfg_aliases! {
        // Platforms
        // windows: { target_os = "windows" },
        linux: { target_os = "linux" },
        macos: { target_os = "macos" },
        android: { target_os = "android" },
        vita: { target_os= "vita" },
        switch: { all(target_os = "horizon", target_arch = "aarch64") },

        // Graphic Backends
        vulkan: { any(windows, linux, macos, android) },
        vitagl: { vita },
        switchgl: { switch },

        // Scripting
        enable_debug: { any(windows, linux, macos) },
    }
}
