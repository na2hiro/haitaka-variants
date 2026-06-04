#![cfg_attr(not(feature = "std"), no_std)]
#![doc = include_str!("../README.md")]
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
            feature = "yokonekoneko"
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
            feature = "yokonekoneko"
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
            feature = "yokonekoneko"
        )
    ),
    all(
        feature = "taimen",
        any(
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko"
        )
    ),
    all(
        feature = "haimen",
        any(
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko"
        )
    ),
    all(
        feature = "neko",
        any(feature = "nekoneko", feature = "yokoneko", feature = "yokonekoneko")
    ),
    all(
        feature = "nekoneko",
        any(feature = "yokoneko", feature = "yokonekoneko")
    ),
    all(feature = "yokoneko", feature = "yokonekoneko"),
))]
compile_error!(
    "features `annan`, `anhoku`, `antouzai`, `taimen`, `haimen`, `neko`, `nekoneko`, `yokoneko`, and `yokonekoneko` are mutually exclusive"
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
