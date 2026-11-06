#![allow(missing_docs)]

#[cfg_attr(feature = "small-board", path = "small.rs")]
mod common;
#[cfg(not(any(feature = "qugiy", feature = "small-board")))]
mod magic;

pub use common::*;
#[cfg(not(any(feature = "qugiy", feature = "small-board")))]
pub use magic::*;
