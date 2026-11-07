//#![cfg_attr(not(any(feature = "std", test)), no_std)]
#![cfg_attr(
    not(any(
        feature = "variant-board",
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        feature = "neko",
        feature = "nekoneko",
        feature = "yokoneko",
        feature = "yokonekoneko",
        feature = "tenkyo",
        feature = "tenjiku",
        feature = "anki"
    )),
    doc = include_str!("../README.md")
)]
#![cfg_attr(
    feature = "variant-board",
    doc = "Small-board shogi, with rules selected at compile time.\n```\nuse haitaka::*;\nlet board = Board::startpos();\nassert_eq!(board.to_string(), SelectedRule::STARTPOS);\nassert_eq!(core::mem::size_of::<BitBoard>(), if Square::NUM <= 64 { 8 } else { 16 });\n```"
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
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        )
    ),
    all(
        feature = "anhoku",
        any(
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        )
    ),
    all(
        feature = "antouzai",
        any(
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        )
    ),
    all(
        feature = "taimen",
        any(
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        )
    ),
    all(
        feature = "haimen",
        any(
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        )
    ),
    all(
        feature = "neko",
        any(
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        )
    ),
    all(
        feature = "nekoneko",
        any(
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        )
    ),
    all(
        feature = "yokoneko",
        any(
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        )
    ),
    all(
        feature = "yokonekoneko",
        any(feature = "tenkyo", feature = "tenjiku", feature = "anki")
    ),
    all(feature = "tenkyo", any(feature = "tenjiku", feature = "anki")),
    all(feature = "tenjiku", feature = "anki"),
))]
compile_error!("variant rule features are mutually exclusive");

use haitaka_types::*;

pub use bitboard::*;
pub use color::*;
pub use dfpn::*;
pub use file::*;
pub use haitaka_types::{
    BOARD_HEIGHT, BOARD_WIDTH, BoardGeometry, RuleSpec, SelectedGeometry, SelectedRule,
    fixed_bitboard, geometry, packed_coordinates,
};
pub use history::*;
pub use piece::*;
pub use rank::*;
pub use shogi_move::*;
pub use sliders::*;
pub use square::*;

#[cfg_attr(feature = "compact-board", path = "small_attacks.rs")]
pub mod attacks;
pub mod board;
pub mod dfpn;
pub mod history;
#[cfg_attr(feature = "compact-board", path = "small_slider_moves.rs")]
pub mod slider_moves;
#[cfg(any(
    feature = "annan",
    feature = "anhoku",
    feature = "antouzai",
    feature = "taimen",
    feature = "haimen",
    feature = "neko",
    feature = "nekoneko",
    feature = "yokoneko",
    feature = "yokonekoneko",
    feature = "tenkyo",
    feature = "tenjiku",
    feature = "anki"
))]
pub mod variant_rules;

pub use attacks::*;
pub use board::*;
pub use slider_moves::*;

#[cfg(feature = "extended-rules")]
pub mod extended;

#[cfg(all(
    any(
        feature = "shoshogi",
        feature = "chushogi",
        feature = "classical-rules"
    ),
    any(
        feature = "variant-board",
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        feature = "neko",
        feature = "nekoneko",
        feature = "yokoneko",
        feature = "yokonekoneko",
        feature = "tenkyo",
        feature = "tenjiku",
        feature = "anki"
    )
))]
compile_error!("extended rules cannot be combined with another ruleset");
#[cfg(all(feature = "shoshogi", feature = "chushogi"))]
compile_error!("select exactly one extended ruleset");

#[cfg(feature = "extended-rules")]
const _: () = assert!(
    (cfg!(feature = "shoshogi") as u8
        + cfg!(feature = "chushogi") as u8
        + cfg!(feature = "torishogi") as u8
        + cfg!(feature = "washogi") as u8
        + cfg!(feature = "heian-dai") as u8
        + cfg!(feature = "daishogi") as u8)
        <= 1,
    "select exactly one extended ruleset"
);
