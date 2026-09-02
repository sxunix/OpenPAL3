//! PAL3's combat system, ported 1:1 from the original executable.
//!
//! [`formulas`] is the arithmetic on its own — no state, no rendering —
//! so every expression can be unit-tested against the numbers read out of
//! the binary. [`state`] adds the battle itself: eleven slots, the action
//! gauge, status timers and the win/lose test, still with nothing on
//! screen. [`actions`] is what a combatant can do on its turn, and
//! [`settlement`] what winning pays out, and [`ai`] how an unsteered
//! combatant chooses. The
//! reverse-engineering write-ups live in
//! `switch/re/notes/combat_formulas.md` and `combat_flow.md`.

pub mod actions;
pub mod ai;
pub mod encounter;
pub mod formulas;
pub mod session;
pub mod settlement;
pub mod state;
