from collections import OrderedDict

import chess
import torch
from feature_block import FeatureBlock

import variant

DONOR_SINGLE_BASE_HASH = 0x23627E42
DONOR_SINGLE_MODE_HASHES = {
    "single-behind": DONOR_SINGLE_BASE_HASH ^ 0x9E3779B1,
    "single-front": DONOR_SINGLE_BASE_HASH ^ 0x3C6EF362,
    # Enemy-donor variants (taimen/haimen) reuse the geometry but need distinct
    # block hashes; these must match haitaka_wasm/src/nnue.rs.
    "single-front-enemy": DONOR_SINGLE_BASE_HASH ^ 0x85EBCA77,
    "single-behind-enemy": DONOR_SINGLE_BASE_HASH ^ 0xC2B2AE3D,
}
DONOR_SINGLE_HASH = DONOR_SINGLE_MODE_HASHES.get(
    getattr(variant, "DONOR_MODE", "none"),
    DONOR_SINGLE_BASE_HASH,
)
DONOR_PAIR_HASH = 0x467CDF71
DONOR_KNIGHT8_HASH = 0x3CC37189
NUM_SQ = variant.SQUARES
NUM_PT = variant.PIECE_TYPES
NUM_COLORS = 2
NUM_SINGLE_FEATURES = NUM_SQ * NUM_PT * NUM_COLORS
NUM_PAIR_SLOTS = 2
NUM_PAIR_FEATURES = NUM_SINGLE_FEATURES * NUM_PAIR_SLOTS
NUM_KNIGHT8_SLOTS = 8
NUM_KNIGHT8_FEATURES = NUM_SINGLE_FEATURES * NUM_KNIGHT8_SLOTS


class DonorSingleEff(FeatureBlock):
    def __init__(self):
        super(DonorSingleEff, self).__init__(
            "DonorSingleEff",
            DONOR_SINGLE_HASH,
            OrderedDict([("DonorSingleEff", NUM_SINGLE_FEATURES)]),
        )

    def get_active_features(self, board: chess.Board):
        raise Exception("Use the C++ data loader for donor feature expansion during training")

    def get_initial_psqt_features(self):
        return [0] * self.num_features


class DonorPairSlots(FeatureBlock):
    def __init__(self):
        super(DonorPairSlots, self).__init__(
            "DonorPairSlots",
            DONOR_PAIR_HASH,
            OrderedDict([("DonorPairSlots", NUM_PAIR_FEATURES)]),
        )

    def get_active_features(self, board: chess.Board):
        raise Exception("Use the C++ data loader for donor feature expansion during training")

    def get_initial_psqt_features(self):
        return [0] * self.num_features


class DonorKnight8Slots(FeatureBlock):
    def __init__(self):
        super(DonorKnight8Slots, self).__init__(
            "DonorKnight8Slots",
            DONOR_KNIGHT8_HASH,
            OrderedDict([("DonorKnight8Slots", NUM_KNIGHT8_FEATURES)]),
        )

    def get_active_features(self, board: chess.Board):
        raise Exception("Use the C++ data loader for donor feature expansion during training")

    def get_initial_psqt_features(self):
        return [0] * self.num_features


def get_feature_block_clss():
    return [DonorSingleEff, DonorPairSlots, DonorKnight8Slots]
