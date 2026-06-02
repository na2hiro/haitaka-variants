from feature_set import FeatureSet

import argparse
import donor_features
import halfka
import halfka_v2
import halfkp

_feature_modules = [halfkp, halfka, halfka_v2, donor_features]
_feature_blocks_by_name = {}


def _add_feature_block(feature_block_cls):
    feature_block = feature_block_cls()
    _feature_blocks_by_name[feature_block.name] = feature_block


def _add_features_blocks_from_module(module):
    for feature_block_cls in module.get_feature_block_clss():
        _add_feature_block(feature_block_cls)


def get_feature_block_from_name(name):
    return _feature_blocks_by_name[name]


def get_feature_blocks_from_names(names):
    return [_feature_blocks_by_name[name] for name in names]


def get_feature_set_from_name(name):
    return FeatureSet(get_feature_blocks_from_names(name.split("+")))


def get_available_feature_blocks_names():
    return list(iter(_feature_blocks_by_name))


def add_argparse_args(parser):
    default_feature_set_name = "HalfKAv2^"
    parser.add_argument(
        "--features",
        dest="features",
        default=default_feature_set_name,
        help=(
            "The feature set to use. Can be a union of feature blocks. "
            "\"^\" denotes a factorized block. Currently available feature blocks are: "
            + ", ".join(get_available_feature_blocks_names())
        ),
    )


def _init():
    for module in _feature_modules:
        _add_features_blocks_from_module(module)


_init()
