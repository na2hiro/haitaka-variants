#![allow(missing_docs)]

#[cfg_attr(feature = "compact-board", path = "small.rs")]
mod common;
#[cfg(not(any(feature = "qugiy", feature = "compact-board")))]
mod magic;

pub use common::*;
#[cfg(not(any(feature = "qugiy", feature = "compact-board")))]
pub use magic::*;
