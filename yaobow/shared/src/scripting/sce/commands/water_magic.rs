//! 152 在玩家角色周身渲染水特效 — `LAB_00438b60`.
//!
//! The handler is one call: `FUN_00410840(param)`. Non-zero builds a
//! `gbParticleEffect`, loads `effect\gbParticle\slht.par` into it and
//! sets its field `+0x46` to 8.0; zero destroys it (0x1002bXXX). The
//! effect is drawn at the player, which is where the "周身" in the
//! opcode's own description comes from.
//!
//! `slht.par` is one group named `shenshu` drawing `partexture\huo3.jpg`
//! additively — see `switch/re/notes/par_format.md` for the format and
//! `openpal3::scene::particles` for the simulation.

use crate::openpal3::directors::SceneManagerExtensions;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;

/// The file `FUN_00410840` loads.
pub const WATER_MAGIC_PAR: &str = "slht.par";

/// 152 WaterMagic.
#[derive(Debug, Clone)]
pub struct SceCommandWaterMagic {
    on: i32,
}

impl SceCommand for SceCommandWaterMagic {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        state.set_water_magic(self.on != 0);
        true
    }
}

impl SceCommandWaterMagic {
    pub fn new(on: i32) -> Self {
        Self { on }
    }
}
