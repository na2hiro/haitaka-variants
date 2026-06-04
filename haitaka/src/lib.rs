//#![cfg_attr(not(any(feature = "std", test)), no_std)]
#![cfg_attr(
    not(any(feature = "annan", feature = "anhoku", feature = "antouzai", feature = "taimen", feature = "haimen")),
    doc = include_str!("../README.md")
)]

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

#[cfg(any(
    all(
        feature = "annan",
        any(
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen"
        )
    ),
    all(
        feature = "anhoku",
        any(feature = "antouzai", feature = "taimen", feature = "haimen")
    ),
    all(feature = "antouzai", any(feature = "taimen", feature = "haimen")),
    all(feature = "taimen", feature = "haimen"),
))]
compile_error!(
    "features `annan`, `anhoku`, `antouzai`, `taimen`, and `haimen` are mutually exclusive"
);

use haitaka_types::*;

pub use bitboard::*;
pub use color::*;
pub use dfpn::*;
pub use file::*;
pub use piece::*;
pub use rank::*;
pub use shogi_move::*;
pub use sliders::*;
pub use square::*;

pub mod attacks;
pub mod board;
pub mod dfpn;
pub mod slider_moves;
#[cfg(any(
    feature = "annan",
    feature = "anhoku",
    feature = "antouzai",
    feature = "taimen",
    feature = "haimen"
))]
pub mod variant_rules;

pub use attacks::*;
pub use board::*;
pub use slider_moves::*;
