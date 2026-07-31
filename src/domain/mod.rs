//! The vocabulary of the problem: money, pools, heights and networks.
//!
//! These types have no dependencies on any other module in the crate and perform no I/O.
//! Anything that does not model a core domain concept does not belong here.

pub mod height;
pub mod network;
pub mod pool;
pub mod zatoshi;
