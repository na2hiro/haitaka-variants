//#![cfg_attr(not(any(feature = "std", test)), no_std)]
#![cfg_attr(not(feature = "annan"), doc = include_str!("../README.md"))]

//! # Examples
//!
//! This crate also includes some examples that illustrate the API and can be used as tools. You can find them in the
//! `examples` directory of the repository:
//!
//! - [Find Magics](https://github.com/tofutofu/haitaka/tree/main/haitaka/examples/find_magics.rs) Generates magic numbers for slider moves.
//! - [Perft](https://github.com/tofutofu/haitaka/tree/main/haitaka/examples/perft.rs) A perft implementation for Shogi.
//!
//! To run an example, clone the reposity and use one of the following commands:
//!
//! ```bash
//! cargo run --release --example find_magics -- --verbose
//! cargo run --release --example perft -- 3
//! ```

use haitaka_types::*;

pub use bitboard::*;
pub use color::*;
pub use file::*;
pub use piece::*;
pub use rank::*;
pub use shogi_move::*;
pub use sliders::*;
pub use square::*;

pub mod attacks;
#[cfg(feature = "annan")]
pub mod annan;
pub mod board;
pub mod slider_moves;

pub use attacks::*;
pub use board::*;
pub use slider_moves::*;
