//! Java library behaviour reproduced bit for bit, so tiles rendered here match tiles rendered by BlueMap.

pub mod math;
mod random;
pub mod trig;

pub use random::{JavaRandom, string_hash};
