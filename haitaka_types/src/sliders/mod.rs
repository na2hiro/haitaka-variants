#![allow(missing_docs)]

#[cfg_attr(feature = "minishogi", path = "small.rs")]
mod common;
#[cfg(not(any(feature = "qugiy", feature = "minishogi")))]
mod magic;

pub use common::*;
#[cfg(not(any(feature = "qugiy", feature = "minishogi")))]
pub use magic::*;
