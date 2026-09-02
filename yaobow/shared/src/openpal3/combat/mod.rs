//! PAL3's combat system, ported 1:1 from the original executable.
//!
//! Phase 1 is [`formulas`] — the arithmetic on its own, with no state and
//! no rendering, so every expression can be unit-tested against the
//! numbers read out of the binary. The reverse-engineering write-ups live
//! in `switch/re/notes/combat_formulas.md` and `combat_flow.md`.

pub mod formulas;
