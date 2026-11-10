typedef int step12_int32;
typedef long long step12_int64;

static_assert(sizeof(step12_int32) == 4, "Step12 requires a signed 32-bit int");
static_assert(sizeof(step12_int64) == 8, "Step12 requires a signed 64-bit long long");

extern "C" {

static const step12_int64 STEP12_INVALID_KEY = 9223372036854775807LL;

__global__ void step12_row_metadata(
    const step12_int32* feature_indices,
    const float* feature_values,
    step12_int32* active_counts,
    step12_int32* invalid,
    step12_int32 batch_size,
    step12_int32 max_active_features,
    step12_int32 feature_count) {
    const step12_int32 row = static_cast<step12_int32>(blockIdx.x * blockDim.x + threadIdx.x);
    if (row >= batch_size) return;

    const step12_int32* indices = feature_indices + row * max_active_features;
    const float* values = feature_values + row * max_active_features;
    step12_int32 count = max_active_features;
    for (step12_int32 slot = 0; slot < max_active_features; ++slot) {
        const step12_int32 feature = indices[slot];
        if (feature == -1) {
            count = slot;
            break;
        }
        if (feature < 0 || feature >= feature_count || !isfinite(values[slot])) {
            atomicExch(invalid, 1);
        }
    }
    active_counts[row] = count;
}

__global__ void step12_build_keys(
    const step12_int32* feature_indices,
    const step12_int32* active_counts,
    step12_int64* keys,
    step12_int32 batch_size,
    step12_int32 max_active_features,
    step12_int64 stride) {
    const step12_int64 ordinal = static_cast<step12_int64>(blockIdx.x) * blockDim.x + threadIdx.x;
    const step12_int64 total = static_cast<step12_int64>(batch_size) * max_active_features;
    if (ordinal >= total) return;

    const step12_int32 row = static_cast<step12_int32>(ordinal / max_active_features);
    const step12_int32 slot = static_cast<step12_int32>(ordinal % max_active_features);
    if (slot >= active_counts[row]) {
        keys[ordinal] = STEP12_INVALID_KEY;
        return;
    }
    const step12_int32 feature = feature_indices[ordinal];
    keys[ordinal] = static_cast<step12_int64>(feature) * stride + ordinal;
}

__global__ void step12_segments(
    const step12_int64* sorted_keys,
    step12_int32* segment_start,
    step12_int32* segment_end,
    step12_int32 record_count,
    step12_int32 feature_count,
    step12_int64 stride) {
    const step12_int32 ordinal = static_cast<step12_int32>(blockIdx.x * blockDim.x + threadIdx.x);
    if (ordinal >= record_count) return;

    const step12_int64 key = sorted_keys[ordinal];
    if (key == STEP12_INVALID_KEY) return;
    const step12_int32 feature = static_cast<step12_int32>(key / stride);
    if (feature < 0 || feature >= feature_count) return;

    const bool first = ordinal == 0 || sorted_keys[ordinal - 1] / stride != feature;
    const bool last = ordinal + 1 == record_count || sorted_keys[ordinal + 1] == STEP12_INVALID_KEY || sorted_keys[ordinal + 1] / stride != feature;
    if (first) segment_start[feature] = ordinal;
    if (last) segment_end[feature] = ordinal + 1;
}

__global__ void step12_reduce_weights(
    const step12_int64* sorted_keys,
    const step12_int32* segment_start,
    const step12_int32* segment_end,
    const float* feature_values,
    const float* output_grad,
    float* weight_grad,
    step12_int32 feature_count,
    step12_int32 output_size,
    step12_int32 max_active_features,
    step12_int64 stride) {
    const step12_int32 feature = static_cast<step12_int32>(blockIdx.x);
    const step12_int32 channel = static_cast<step12_int32>(blockIdx.y * blockDim.x + threadIdx.x);
    if (feature >= feature_count || channel >= output_size) return;

    float accumulator = weight_grad[static_cast<step12_int64>(feature) * output_size + channel];
    const step12_int32 begin = segment_start[feature];
    const step12_int32 end = segment_end[feature];
    if (begin >= 0 && end >= begin) {
        for (step12_int32 sorted = begin; sorted < end; ++sorted) {
            const step12_int64 packed = sorted_keys[sorted];
            const step12_int64 ordinal = packed % stride;
            const step12_int32 row = static_cast<step12_int32>(ordinal / max_active_features);
            const step12_int32 slot = static_cast<step12_int32>(ordinal % max_active_features);
            const float upstream = output_grad[static_cast<step12_int64>(row) * output_size + channel];
            if (upstream != 0.0f) {
                const float value = feature_values[ordinal];
                const float product = __fmul_rn(upstream, value);
                accumulator = __fadd_rn(accumulator, product);
            }
        }
    }
    weight_grad[static_cast<step12_int64>(feature) * output_size + channel] = accumulator;
}

__global__ void step12_reduce_bias(
    const float* output_grad,
    float* bias_grad,
    step12_int32 batch_size,
    step12_int32 output_size) {
    const step12_int32 channel = static_cast<step12_int32>(blockIdx.x * blockDim.x + threadIdx.x);
    if (channel >= output_size) return;

    float accumulator = bias_grad[channel];
    for (step12_int32 row = 0; row < batch_size; ++row) {
        const float upstream = output_grad[static_cast<step12_int64>(row) * output_size + channel];
        if (upstream != 0.0f) accumulator = __fadd_rn(accumulator, upstream);
    }
    bias_grad[channel] = accumulator;
}

}
