#![cfg_attr(not(feature = "std"), no_std)]
#![doc = include_str!("../README.md")]
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
pub mod bitboard;
pub mod color;
pub mod file;
pub mod helpers;
pub mod piece;
pub mod rank;
pub mod shogi_move;
pub mod sliders;
pub mod square;

pub use bitboard::*;
pub use color::*;
pub use file::*;
pub use piece::*;
pub use rank::*;
pub use shogi_move::*;
pub use sliders::*;
pub use square::*;
