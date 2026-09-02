//! Reading and writing script variables.
//!
//! A variable index is signed: a negative one names a **global** that
//! lives in the save file, a non-negative one a **local** of the running
//! proc. `SceCommandCmp` (opcodes 6–11) already branches on the sign, and
//! everything that touches a variable has to agree with it or the two
//! halves of a comparison end up reading different storage.

use crate::scripting::sce::SceState;

pub fn read(state: &mut SceState, var: i16) -> i32 {
    if var < 0 {
        state
            .global_state_mut()
            .persistent_state_mut()
            .get_global(var)
            .unwrap_or(0)
    } else {
        state.context_mut().get_local(var).unwrap_or(0)
    }
}

pub fn write(state: &mut SceState, var: i16, value: i32) {
    if var < 0 {
        state
            .global_state_mut()
            .persistent_state_mut()
            .set_global(var, value);
    } else {
        state.context_mut().set_local(var, value);
    }
}
