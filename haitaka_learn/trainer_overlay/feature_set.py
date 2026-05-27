from feature_block import FeatureBlock
import torch

PSQT_BUCKETS = 8
LS_BUCKETS = 8


def _calculate_features_hash(features):
    if len(features) == 1:
        return features[0].hash

    tail_hash = _calculate_features_hash(features[1:])
    return (features[0].hash ^ (tail_hash << 1) ^ (tail_hash >> 1)) & 0xFFFFFFFF


class FeatureSet:
    def __init__(self, features):
        for feature in features:
            if not isinstance(feature, FeatureBlock):
                raise Exception("All features must subclass FeatureBlock")

        self.features = features
        self.hash = _calculate_features_hash(features)
        self.name = "+".join(feature.name for feature in features)
        self.num_real_features = sum(feature.num_real_features for feature in features)
        self.num_virtual_features = sum(feature.num_virtual_features for feature in features)
        self.num_features = sum(feature.num_features for feature in features)
        self.num_psqt_buckets = PSQT_BUCKETS
        self.num_ls_buckets = LS_BUCKETS

    def get_virtual_feature_ranges(self):
        ranges = []
        offset = 0
        for feature in self.features:
            if feature.num_virtual_features:
                ranges.append((offset + feature.num_real_features, offset + feature.num_features))
            offset += feature.num_features
        return ranges

    def get_real_feature_ranges(self):
        ranges = []
        offset = 0
        for feature in self.features:
            ranges.append((offset, offset + feature.num_real_features))
            offset += feature.num_features
        return ranges

    def get_active_features(self, board):
        blocks = [feature.get_active_features(board) for feature in self.features]
        if all(
            w_local.ndim == 1 and w_local.numel() == feature.num_features
            for feature, (w_local, _) in zip(self.features, blocks)
        ):
            white = torch.cat([w_local for (w_local, _) in blocks])
            black = torch.cat([b_local for (_, b_local) in blocks])
            return white, black

        white = torch.zeros(0, dtype=torch.int64)
        black = torch.zeros(0, dtype=torch.int64)
        offset = 0
        for feature, (w_local, b_local) in zip(self.features, blocks):
            white = torch.cat([white, w_local.to(dtype=torch.int64) + offset])
            black = torch.cat([black, b_local.to(dtype=torch.int64) + offset])
            offset += feature.num_features
        return white, black

    def get_feature_factors(self, idx):
        offset = 0
        for feature in self.features:
            if idx < offset + feature.num_real_features:
                return [offset + i for i in feature.get_feature_factors(idx - offset)]
            offset += feature.num_features
        raise Exception("No feature block to factorize {}".format(idx))

    def get_virtual_to_real_features_gather_indices(self):
        indices = []
        offset = 0
        for feature in self.features:
            for i_real in range(feature.num_real_features):
                i_fact = feature.get_feature_factors(i_real)
                indices.append([offset + i for i in i_fact])
            offset += feature.num_features
        return indices

    def get_initial_psqt_features(self):
        init = []
        for feature in self.features:
            init += feature.get_initial_psqt_features()
        return init
