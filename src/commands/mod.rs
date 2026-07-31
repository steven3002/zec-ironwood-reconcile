//! Thin orchestration for each CLI verb.
//!
//! Commands wire the pure core to the outside world. They contain no accounting logic, so
//! that the behaviour of `reconcile` and of offline `verify` cannot diverge.

pub mod inspect;
pub mod verify;
