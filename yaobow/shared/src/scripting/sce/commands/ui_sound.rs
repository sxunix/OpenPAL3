//! 界面按钮音效。原版统一走 `FUN_0053df70(名字, 1)` + `FUN_0053e2e0` 放一次，名字从各模块
//! 自己的一张字符串指针表里取（买卖页 `0x5b46b4` 起、冶炼 `0x5bc90c` 起、当铺 `0x5dff88` 起）。
//! 用法按调用点前后的分支认的，见 `switch/re/notes/ui_sound.md`。

use std::cell::RefCell;
use std::rc::Rc;

use crate::scripting::sce::SceState;
use radiance::audio::Codec;

/// 普通点击。
pub const CLICK: &str = "wa001";
/// 弹提示框（当铺红利、冶炼放上材料）。
pub const NOTE: &str = "wa003";
/// 点了置灰的钮 / 做不到。
pub const DENIED: &str = "wa004";
/// 拖动条、加减数量动了一格。
pub const TICK: &str = "wa005";
/// 冶炼开炉（5.1 秒）。
pub const FORGE: &str = "wa026";

/// 放一次，不循环；音源交给全局表保活。
pub(super) fn play(state: &mut SceState, name: &str) {
    match state.asset_mgr().load_snd_data(name) {
        Ok(d) => {
            let mut source = state.audio_engine().create_source();
            source.set_data(d, Codec::Wav);
            source.play(false);
            state
                .global_state_mut()
                .add_sound_source(Rc::new(RefCell::new(source)));
        }
        Err(e) => log::warn!("界面音效 {name} 读不出来: {e:?}"),
    }
}
