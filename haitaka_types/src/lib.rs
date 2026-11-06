#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(not(feature = "minishogi"), doc = include_str!("../README.md"))]
#![cfg_attr(
    feature = "minishogi",
    doc = "Compact 5x5 shogi types.\n```\nuse haitaka_types::*;\nassert_eq!(Square::NUM, 25);\nassert_eq!(core::mem::size_of::<BitBoard>(), 8);\n```"
)]
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
pub mod bitboard;
pub mod color;
#[cfg_attr(feature = "minishogi", path = "minishogi/file.rs")]
pub mod file;
pub mod helpers;
pub mod piece;
#[cfg_attr(feature = "minishogi", path = "minishogi/rank.rs")]
pub mod rank;
pub mod shogi_move;
pub mod sliders;
#[cfg_attr(feature = "minishogi", path = "minishogi/square.rs")]
pub mod square;

pub use bitboard::*;
pub use color::*;
pub use file::*;
pub use piece::*;
pub use rank::*;
pub use shogi_move::*;
pub use sliders::*;
pub use square::*;

pub mod fixed_bitboard;
pub mod geometry;
pub use geometry::{
    BOARD_HEIGHT, BOARD_WIDTH, BoardGeometry, RuleSpec, SelectedGeometry, SelectedRule,
};

#[cfg(all(
    feature = "minishogi",
    any(
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
compile_error!("minishogi and movement-donor rule features are mutually exclusive");

pub mod packed_coordinates;

const _: () = {
    assert!(Square::NUM == SelectedGeometry::SQUARES);
    assert!(File::NUM == BOARD_WIDTH);
    assert!(Rank::NUM == BOARD_HEIGHT);
    assert!(
        core::mem::size_of::<BitBoard>()
            == core::mem::size_of::<<SelectedGeometry as BoardGeometry>::Bits>()
    );
};
