#include <algorithm>
#include <deque>
#include <future>
#include <iostream>
#include <memory>
#include <mutex>
#include <random>
#include <string>
#include <thread>

#include "lib/nnue_training_data_formats.h"
#include "lib/nnue_training_data_stream.h"
#include "lib/rng.h"

#if defined(__x86_64__)
#define EXPORT
#define CDECL
#else
#if defined(_MSC_VER)
#define EXPORT __declspec(dllexport)
#define CDECL __cdecl
#else
#define EXPORT
#define CDECL __attribute__((__cdecl__))
#endif
#endif

using namespace bin;
using namespace chess;

static constexpr int MAX_PIECES = PIECE_COUNT;
static constexpr int MAX_HAND_PIECES = POCKETS ? 2 * static_cast<int>(File::FILE_NB) : 0;
static constexpr int DONOR_PAIR_SLOTS = 2;
static constexpr int DONOR_KNIGHT8_SLOTS = 8;

#ifndef HAITAKA_DONOR_MODE
#define HAITAKA_DONOR_MODE 0
#endif

static Square orient(Color color, Square sq) {
    if (color == Color::White) {
        return sq;
    }
    return flip_horizontally(flip_vertically(sq));
}

static Square orient_flip(Color color, Square sq) {
    if (sq == Square::NB) {
        return Square::MIN;
    }
    if (color == Color::White) {
        return sq;
    }
    return flip_vertically(sq);
}

static int map_king(Square sq) {
    if (Square::KNB == Square(9) && Square::KNB != Square::NB) {
        return (int(sq) - 6 * (int(sq) / int(File::FILE_NB)) - 3) % int(Square::KNB);
    }
    return int(sq) % int(Square::KNB);
}

static Square offset_square(Square sq, int file_delta, int rank_delta) {
    const int file = int(sq) % FILES;
    const int rank = int(sq) / FILES;
    const int next_file = file + file_delta;
    const int next_rank = rank + rank_delta;
    if (next_file < 0 || next_file >= FILES || next_rank < 0 || next_rank >= RANKS) {
        return Square::NB;
    }
    return Square(next_rank * FILES + next_file);
}

static Square offset_square_from_relative(Color color, Square sq, int left, int forward) {
    const int file_delta = color == Color::White ? left : -left;
    const int rank_delta = color == Color::White ? -forward : forward;
    return offset_square(sq, file_delta, rank_delta);
}

static int donor_piece_index(Color perspective, Piece donor_piece) {
    return static_cast<int>(type_of(donor_piece))
        + (color_of(donor_piece) != perspective) * PIECE_TYPES;
}

static Piece friendly_piece_at(const Position& pos, Color color, Square sq) {
    if (sq == Square::NB) {
        return Piece::None;
    }
    const Piece piece = pos.pieceAt(sq);
    if (piece == Piece::None || color_of(piece) != color) {
        return Piece::None;
    }
    return piece;
}

static Square single_donor_square(Color color, Square sq) {
#if HAITAKA_DONOR_MODE == 1
    return color == Color::White ? offset_square(sq, 0, -1) : offset_square(sq, 0, 1);
#elif HAITAKA_DONOR_MODE == 2
    return color == Color::White ? offset_square(sq, 0, 1) : offset_square(sq, 0, -1);
#else
    return Square::NB;
#endif
}

static std::array<Square, DONOR_PAIR_SLOTS> pair_donor_squares(Square sq) {
    return {offset_square(sq, -1, 0), offset_square(sq, 1, 0)};
}

static std::array<Square, DONOR_KNIGHT8_SLOTS> knight8_donor_squares(Color color, Square sq) {
    return {
        offset_square_from_relative(color, sq, 1, 2),
        offset_square_from_relative(color, sq, -1, 2),
        offset_square_from_relative(color, sq, -2, 1),
        offset_square_from_relative(color, sq, -2, -1),
        offset_square_from_relative(color, sq, -1, -2),
        offset_square_from_relative(color, sq, 1, -2),
        offset_square_from_relative(color, sq, 2, -1),
        offset_square_from_relative(color, sq, 2, 1),
    };
}

struct HalfKP {
    static constexpr int NUM_SQ = static_cast<int>(Square::NB);
    static constexpr int NUM_PT = static_cast<int>(PieceType::MaxPiece) * 2;
    static constexpr int NUM_PLANES = NUM_SQ * NUM_PT + 1;
    static constexpr int INPUTS = NUM_PLANES * NUM_SQ;
    static constexpr int MAX_ACTIVE_FEATURES = MAX_PIECES;

    static int feature_index(Color color, Square ksq, Square sq, Piece p) {
        auto p_idx = static_cast<int>(type_of(p)) * 2 + (color_of(p) != color);
        return 1 + static_cast<int>(orient(color, sq)) + p_idx * NUM_SQ
            + map_king(ksq) * NUM_PLANES;
    }

    static std::pair<int, int> fill_features_sparse(
        const TrainingDataEntry& e, int* features, float* values, Color color
    ) {
        auto& pos = e.pos;
        auto ksq = pos.kingSquare(color);

        int j = 0;
        for (Square sq = Square::MIN; sq <= Square::MAX; ++sq) {
            const auto p = pos.pieceAt(sq);
            if (p == Piece::None || type_of(p) == PieceType::King) {
                continue;
            }
            values[j] = 1.0f;
            features[j] = feature_index(color, orient(color, ksq), sq, p);
            ++j;
        }

        return {j, INPUTS};
    }
};

struct HalfKPFactorized {
    static constexpr int K_INPUTS = HalfKP::NUM_SQ;
    static constexpr int PIECE_INPUTS = HalfKP::NUM_SQ * HalfKP::NUM_PT;
    static constexpr int INPUTS = HalfKP::INPUTS + K_INPUTS + PIECE_INPUTS;
    static constexpr int MAX_ACTIVE_FEATURES = HalfKP::MAX_ACTIVE_FEATURES + 1 + MAX_PIECES;

    static std::pair<int, int> fill_features_sparse(
        const TrainingDataEntry& e, int* features, float* values, Color color
    ) {
        const auto [start_j, offset] = HalfKP::fill_features_sparse(e, features, values, color);
        auto& pos = e.pos;

        int j = start_j;
        const auto ksq = pos.kingSquare(color);
        features[j] = offset + static_cast<int>(orient(color, ksq));
        values[j] = static_cast<float>(start_j);
        ++j;

        for (Square sq = Square::MIN; sq <= Square::MAX; ++sq) {
            const auto p = pos.pieceAt(sq);
            if (p == Piece::None || type_of(p) == PieceType::King) {
                continue;
            }
            const auto p_idx = static_cast<int>(type_of(p)) * 2 + (color_of(p) != color);
            values[j] = 1.0f;
            features[j] = offset + K_INPUTS + (p_idx * HalfKP::NUM_SQ) + static_cast<int>(orient(color, sq));
            ++j;
        }

        return {j, INPUTS};
    }
};

struct HalfKA {
    static constexpr int NUM_SQ = static_cast<int>(Square::NB);
    static constexpr int NUM_PT = (static_cast<int>(PieceType::MaxPiece) + 1) * 2;
    static constexpr int NUM_PLANES = NUM_SQ * NUM_PT + 1;
    static constexpr int INPUTS = NUM_PLANES * NUM_SQ;
    static constexpr int MAX_ACTIVE_FEATURES = MAX_PIECES;

    static int feature_index(Color color, Square ksq, Square sq, Piece p) {
        auto p_idx = static_cast<int>(type_of(p)) * 2 + (color_of(p) != color);
        return 1 + static_cast<int>(orient_flip(color, sq)) + p_idx * NUM_SQ
            + map_king(ksq) * NUM_PLANES;
    }

    static std::pair<int, int> fill_features_sparse(
        const TrainingDataEntry& e, int* features, float* values, Color color
    ) {
        auto& pos = e.pos;
        auto ksq = pos.kingSquare(color);

        int j = 0;
        for (Square sq = Square::MIN; sq <= Square::MAX; ++sq) {
            const auto p = pos.pieceAt(sq);
            if (p == Piece::None) {
                continue;
            }
            values[j] = 1.0f;
            features[j] = feature_index(color, orient_flip(color, ksq), sq, p);
            ++j;
        }

        return {j, INPUTS};
    }
};

struct HalfKAFactorized {
    static constexpr int PIECE_INPUTS = HalfKA::NUM_SQ * HalfKA::NUM_PT;
    static constexpr int INPUTS = HalfKA::INPUTS + PIECE_INPUTS;
    static constexpr int MAX_ACTIVE_FEATURES = HalfKA::MAX_ACTIVE_FEATURES + MAX_PIECES;

    static std::pair<int, int> fill_features_sparse(
        const TrainingDataEntry& e, int* features, float* values, Color color
    ) {
        const auto [start_j, offset] = HalfKA::fill_features_sparse(e, features, values, color);
        auto& pos = e.pos;

        int j = start_j;
        for (Square sq = Square::MIN; sq <= Square::MAX; ++sq) {
            const auto p = pos.pieceAt(sq);
            if (p == Piece::None) {
                continue;
            }
            const auto p_idx = static_cast<int>(type_of(p)) * 2 + (color_of(p) != color);
            values[j] = 1.0f;
            features[j] = offset + (p_idx * HalfKA::NUM_SQ) + static_cast<int>(orient_flip(color, sq));
            ++j;
        }

        return {j, INPUTS};
    }
};

struct HalfKAv2 {
    static constexpr int NUM_KSQ = static_cast<int>(Square::KNB);
    static constexpr int NUM_SQ = static_cast<int>(Square::NB);
    static constexpr int NUM_PT = (static_cast<int>(PieceType::MaxPiece) + 1) * 2 - (NUM_KSQ > 1);
    static constexpr int NUM_PLANES =
        NUM_SQ * NUM_PT + MAX_HAND_PIECES * (NUM_PT - (NUM_KSQ > 1));
    static constexpr int INPUTS = NUM_PLANES * NUM_KSQ;
    static constexpr int MAX_ACTIVE_FEATURES = MAX_PIECES + MAX_HAND_PIECES;

    static int feature_index(Color color, Square ksq, Square sq, Piece p) {
        auto p_idx = static_cast<int>(type_of(p)) * 2 + (color_of(p) != color);
        if (NUM_PT % 2 && p_idx == NUM_PT) {
            --p_idx;
        }
        return static_cast<int>(orient_flip(color, sq)) + p_idx * NUM_SQ
            + map_king(ksq) * NUM_PLANES;
    }

    static int feature_index(Color color, Square ksq, int hand_count, Piece p) {
        auto p_idx = static_cast<int>(type_of(p)) * 2 + (color_of(p) != color);
        return hand_count + p_idx * MAX_HAND_PIECES + NUM_SQ * NUM_PT
            + map_king(ksq) * NUM_PLANES;
    }

    static std::pair<int, int> fill_features_sparse(
        const TrainingDataEntry& e, int* features, float* values, Color color
    ) {
        auto& pos = e.pos;
        auto ksq = pos.kingSquare(color);

        int j = 0;
        for (Square sq = Square::MIN; sq <= Square::MAX; ++sq) {
            const auto p = pos.pieceAt(sq);
            if (p == Piece::None) {
                continue;
            }
            values[j] = 1.0f;
            features[j] = feature_index(color, orient_flip(color, ksq), sq, p);
            ++j;
        }

        for (PieceType pt = PieceType::Pawn; pt < PieceType::King; ++pt) {
            for (Color c : {Color::White, Color::Black}) {
                for (int i = 0; i < pos.getHandCount(make_piece(pt, c)); ++i) {
                    values[j] = 1.0f;
                    features[j] = feature_index(color, orient_flip(color, ksq), i, make_piece(pt, c));
                    ++j;
                }
            }
        }

        return {j, INPUTS};
    }
};

struct HalfKAv2Factorized {
    static constexpr int NUM_PT = (static_cast<int>(PieceType::MaxPiece) + 1) * 2;
    static constexpr int INPUTS =
        HalfKAv2::INPUTS
        + HalfKAv2::NUM_SQ * NUM_PT
        + MAX_HAND_PIECES * (NUM_PT - 2 * (HalfKAv2::NUM_KSQ > 1));
    static constexpr int MAX_ACTIVE_FEATURES = HalfKAv2::MAX_ACTIVE_FEATURES + MAX_PIECES;

    static std::pair<int, int> fill_features_sparse(
        const TrainingDataEntry& e, int* features, float* values, Color color
    ) {
        const auto [start_j, offset] = HalfKAv2::fill_features_sparse(e, features, values, color);
        auto& pos = e.pos;

        int j = start_j;
        for (Square sq = Square::MIN; sq <= Square::MAX; ++sq) {
            const auto p = pos.pieceAt(sq);
            if (p == Piece::None) {
                continue;
            }
            auto p_idx = static_cast<int>(type_of(p)) * 2 + (color_of(p) != color);
            values[j] = 1.0f;
            features[j] = offset + (p_idx * HalfKAv2::NUM_SQ) + static_cast<int>(orient_flip(color, sq));
            ++j;
        }

        for (PieceType pt = PieceType::Pawn; pt < PieceType::King; ++pt) {
            for (Color c : {Color::White, Color::Black}) {
                for (int i = 0; i < pos.getHandCount(make_piece(pt, c)); ++i) {
                    values[j] = 1.0f;
                    const auto p_idx = static_cast<int>(pt) * 2 + (c != color);
                    features[j] =
                        offset + i + p_idx * MAX_HAND_PIECES + HalfKAv2::NUM_SQ * NUM_PT;
                    ++j;
                }
            }
        }

        return {j, INPUTS};
    }
};

struct DonorSingleEff {
    static constexpr int INPUTS = FILES * RANKS * PIECE_TYPES * 2;
    static constexpr int MAX_ACTIVE_FEATURES = MAX_PIECES;

    static int feature_index(Color perspective, Square sq, Piece donor_piece) {
        return static_cast<int>(orient_flip(perspective, sq))
            + donor_piece_index(perspective, donor_piece) * FILES * RANKS;
    }

    static std::pair<int, int> fill_features_sparse(
        const TrainingDataEntry& e, int* features, float* values, Color perspective
    ) {
        auto& pos = e.pos;
        int j = 0;
        for (Square sq = Square::MIN; sq <= Square::MAX; ++sq) {
            const auto p = pos.pieceAt(sq);
            if (p == Piece::None) {
                continue;
            }
            const auto donor_piece = friendly_piece_at(pos, color_of(p), single_donor_square(color_of(p), sq));
            if (donor_piece == Piece::None) {
                continue;
            }
            values[j] = 1.0f;
            features[j] = feature_index(perspective, sq, donor_piece);
            ++j;
        }
        return {j, INPUTS};
    }
};

struct DonorPairSlots {
    static constexpr int INPUTS = FILES * RANKS * PIECE_TYPES * 2 * DONOR_PAIR_SLOTS;
    static constexpr int MAX_ACTIVE_FEATURES = MAX_PIECES * DONOR_PAIR_SLOTS;

    static int feature_index(Color perspective, Square sq, int slot, Piece donor_piece) {
        return static_cast<int>(orient_flip(perspective, sq))
            + (donor_piece_index(perspective, donor_piece) + slot * PIECE_TYPES * 2) * FILES * RANKS;
    }

    static std::pair<int, int> fill_features_sparse(
        const TrainingDataEntry& e, int* features, float* values, Color perspective
    ) {
        auto& pos = e.pos;
        int j = 0;
        for (Square sq = Square::MIN; sq <= Square::MAX; ++sq) {
            const auto p = pos.pieceAt(sq);
            if (p == Piece::None) {
                continue;
            }
            const auto donors = pair_donor_squares(sq);
            for (int slot = 0; slot < DONOR_PAIR_SLOTS; ++slot) {
                const auto donor_piece = friendly_piece_at(pos, color_of(p), donors[slot]);
                if (donor_piece == Piece::None) {
                    continue;
                }
                values[j] = 1.0f;
                features[j] = feature_index(perspective, sq, slot, donor_piece);
                ++j;
            }
        }
        return {j, INPUTS};
    }
};

struct DonorKnight8Slots {
    static constexpr int INPUTS = FILES * RANKS * PIECE_TYPES * 2 * DONOR_KNIGHT8_SLOTS;
    static constexpr int MAX_ACTIVE_FEATURES = MAX_PIECES * DONOR_KNIGHT8_SLOTS;

    static int feature_index(Color perspective, Square sq, int slot, Piece donor_piece) {
        return static_cast<int>(orient_flip(perspective, sq))
            + (donor_piece_index(perspective, donor_piece) + slot * PIECE_TYPES * 2) * FILES * RANKS;
    }

    static std::pair<int, int> fill_features_sparse(
        const TrainingDataEntry& e, int* features, float* values, Color perspective
    ) {
        auto& pos = e.pos;
        int j = 0;
        for (Square sq = Square::MIN; sq <= Square::MAX; ++sq) {
            const auto p = pos.pieceAt(sq);
            if (p == Piece::None) {
                continue;
            }
            const auto donors = knight8_donor_squares(color_of(p), sq);
            for (int slot = 0; slot < DONOR_KNIGHT8_SLOTS; ++slot) {
                const auto donor_piece = friendly_piece_at(pos, color_of(p), donors[slot]);
                if (donor_piece == Piece::None) {
                    continue;
                }
                values[j] = 1.0f;
                features[j] = feature_index(perspective, sq, slot, donor_piece);
                ++j;
            }
        }
        return {j, INPUTS};
    }
};

template <typename T, typename... Ts>
struct FeatureSet;

template <typename T>
struct FeatureSet<T> {
    static constexpr int INPUTS = T::INPUTS;
    static constexpr int MAX_ACTIVE_FEATURES = T::MAX_ACTIVE_FEATURES;

    static std::pair<int, int> fill_features_sparse(
        const TrainingDataEntry& e, int* features, float* values, Color color
    ) {
        return T::fill_features_sparse(e, features, values, color);
    }
};

template <typename T, typename U, typename... Ts>
struct FeatureSet<T, U, Ts...> {
    static constexpr int INPUTS = T::INPUTS + FeatureSet<U, Ts...>::INPUTS;
    static constexpr int MAX_ACTIVE_FEATURES =
        T::MAX_ACTIVE_FEATURES + FeatureSet<U, Ts...>::MAX_ACTIVE_FEATURES;

    static std::pair<int, int> fill_features_sparse(
        const TrainingDataEntry& e, int* features, float* values, Color color
    ) {
        const auto [head_count, _] = T::fill_features_sparse(e, features, values, color);
        const auto [tail_count, __] =
            FeatureSet<U, Ts...>::fill_features_sparse(e, features + head_count, values + head_count, color);
        for (int i = 0; i < tail_count; ++i) {
            features[head_count + i] += T::INPUTS;
        }
        return {head_count + tail_count, INPUTS};
    }
};

struct SparseBatch {
    static constexpr bool IS_BATCH = true;

    template <typename... Ts>
    SparseBatch(FeatureSet<Ts...>, const std::vector<TrainingDataEntry>& entries) {
        num_inputs = FeatureSet<Ts...>::INPUTS;
        size = entries.size();
        is_white = new float[size];
        outcome = new float[size];
        score = new float[size];
        white = new int[size * FeatureSet<Ts...>::MAX_ACTIVE_FEATURES];
        black = new int[size * FeatureSet<Ts...>::MAX_ACTIVE_FEATURES];
        white_values = new float[size * FeatureSet<Ts...>::MAX_ACTIVE_FEATURES];
        black_values = new float[size * FeatureSet<Ts...>::MAX_ACTIVE_FEATURES];
        psqt_indices = new int[size];
        layer_stack_indices = new int[size];

        num_active_white_features = 0;
        num_active_black_features = 0;
        max_active_features = FeatureSet<Ts...>::MAX_ACTIVE_FEATURES;

        for (std::size_t i = 0; i < size * FeatureSet<Ts...>::MAX_ACTIVE_FEATURES; ++i) {
            white[i] = -1;
            black[i] = -1;
            white_values[i] = 0.0f;
            black_values[i] = 0.0f;
        }

        for (int i = 0; i < int(entries.size()); ++i) {
            fill_entry(FeatureSet<Ts...>{}, i, entries[i]);
        }
    }

    int num_inputs;
    int size;
    float* is_white;
    float* outcome;
    float* score;
    int num_active_white_features;
    int num_active_black_features;
    int max_active_features;
    int* white;
    int* black;
    float* white_values;
    float* black_values;
    int* psqt_indices;
    int* layer_stack_indices;

    ~SparseBatch() {
        delete[] is_white;
        delete[] outcome;
        delete[] score;
        delete[] white;
        delete[] black;
        delete[] white_values;
        delete[] black_values;
        delete[] psqt_indices;
        delete[] layer_stack_indices;
    }

private:
    template <typename... Ts>
    void fill_entry(FeatureSet<Ts...>, int i, const TrainingDataEntry& e) {
        is_white[i] = static_cast<float>(e.pos.sideToMove() == Color::White);
        outcome[i] = (e.result + 1.0f) / 2.0f;
        score[i] = e.score;
        psqt_indices[i] = (e.pos.pieceCount() - 1) * 8 / MAX_PIECES;
        layer_stack_indices[i] = psqt_indices[i];
        const int offset = i * FeatureSet<Ts...>::MAX_ACTIVE_FEATURES;
        num_active_white_features += FeatureSet<Ts...>::fill_features_sparse(
            e, white + offset, white_values + offset, Color::White
        ).first;
        num_active_black_features += FeatureSet<Ts...>::fill_features_sparse(
            e, black + offset, black_values + offset, Color::Black
        ).first;
    }
};

struct AnyStream {
    virtual ~AnyStream() = default;
};

template <typename StorageT>
struct Stream : AnyStream {
    Stream(int concurrency, const char* filename, bool cyclic, std::function<bool(const TrainingDataEntry&)> skipPredicate)
        : m_stream(training_data::open_sfen_input_file_parallel(concurrency, filename, cyclic, skipPredicate)) {}

    virtual StorageT* next() = 0;

protected:
    std::unique_ptr<training_data::BasicSfenInputStream> m_stream;
};

template <typename FeatureSetT, typename StorageT>
struct FeaturedBatchStream : Stream<StorageT> {
    static_assert(StorageT::IS_BATCH);

    using BaseType = Stream<StorageT>;

    static constexpr int num_feature_threads_per_reading_thread = 2;

    FeaturedBatchStream(
        int concurrency,
        const char* filename,
        int batch_size,
        bool cyclic,
        std::function<bool(const TrainingDataEntry&)> skipPredicate
    )
        : BaseType(
              std::max(1, concurrency / num_feature_threads_per_reading_thread),
              filename,
              cyclic,
              skipPredicate
          ),
          m_concurrency(concurrency),
          m_batch_size(batch_size) {
        m_stop_flag.store(false);

        auto worker = [this]() {
            std::vector<TrainingDataEntry> entries;
            entries.reserve(m_batch_size);

            while (!m_stop_flag.load()) {
                entries.clear();
                {
                    std::unique_lock lock(m_stream_mutex);
                    BaseType::m_stream->fill(entries, m_batch_size);
                    if (entries.empty()) {
                        break;
                    }
                }

                auto batch = new StorageT(FeatureSetT{}, entries);
                {
                    std::unique_lock lock(m_batch_mutex);
                    m_batches_not_full.wait(lock, [this]() {
                        return m_batches.size() < std::size_t(m_concurrency + 1) || m_stop_flag.load();
                    });
                    m_batches.emplace_back(batch);
                    lock.unlock();
                    m_batches_any.notify_one();
                }
            }
            m_num_workers.fetch_sub(1);
            m_batches_any.notify_one();
        };

        const int num_feature_threads =
            std::max(1, concurrency - std::max(1, concurrency / num_feature_threads_per_reading_thread));

        m_num_workers.store(num_feature_threads);
        for (int i = 0; i < num_feature_threads; ++i) {
            m_workers.emplace_back(worker);
        }
    }

    StorageT* next() override {
        std::unique_lock lock(m_batch_mutex);
        m_batches_any.wait(lock, [this]() { return !m_batches.empty() || m_num_workers.load() == 0; });
        if (m_batches.empty()) {
            return nullptr;
        }
        auto batch = m_batches.front();
        m_batches.pop_front();
        lock.unlock();
        m_batches_not_full.notify_one();
        return batch;
    }

    ~FeaturedBatchStream() {
        m_stop_flag.store(true);
        m_batches_not_full.notify_all();
        for (auto& worker : m_workers) {
            if (worker.joinable()) {
                worker.join();
            }
        }
        for (auto& batch : m_batches) {
            delete batch;
        }
    }

private:
    int m_batch_size;
    int m_concurrency;
    std::deque<StorageT*> m_batches;
    std::mutex m_batch_mutex;
    std::mutex m_stream_mutex;
    std::condition_variable m_batches_not_full;
    std::condition_variable m_batches_any;
    std::atomic_bool m_stop_flag{false};
    std::atomic_int m_num_workers{0};
    std::vector<std::thread> m_workers;
};

std::function<bool(const TrainingDataEntry&)> make_skip_predicate(bool filtered, int random_fen_skipping) {
    if (!filtered && !random_fen_skipping) {
        return nullptr;
    }

    return [random_fen_skipping, prob = double(random_fen_skipping) / (random_fen_skipping + 1)](
               const TrainingDataEntry&
           ) {
        if (!random_fen_skipping) {
            return false;
        }
        std::bernoulli_distribution distrib(prob);
        auto& prng = rng::get_thread_local_rng();
        return distrib(prng);
    };
}

extern "C" {

EXPORT Stream<SparseBatch>* CDECL create_sparse_batch_stream(
    const char* feature_set_c,
    int concurrency,
    const char* filename,
    int batch_size,
    bool cyclic,
    bool filtered,
    int random_fen_skipping
) {
    auto skipPredicate = make_skip_predicate(filtered, random_fen_skipping);
    std::string_view feature_set(feature_set_c);

    if (feature_set == "HalfKP") {
        return new FeaturedBatchStream<FeatureSet<HalfKP>, SparseBatch>(
            concurrency, filename, batch_size, cyclic, skipPredicate
        );
    }
    if (feature_set == "HalfKP^") {
        return new FeaturedBatchStream<FeatureSet<HalfKPFactorized>, SparseBatch>(
            concurrency, filename, batch_size, cyclic, skipPredicate
        );
    }
    if (feature_set == "HalfKA") {
        return new FeaturedBatchStream<FeatureSet<HalfKA>, SparseBatch>(
            concurrency, filename, batch_size, cyclic, skipPredicate
        );
    }
    if (feature_set == "HalfKA^") {
        return new FeaturedBatchStream<FeatureSet<HalfKAFactorized>, SparseBatch>(
            concurrency, filename, batch_size, cyclic, skipPredicate
        );
    }
    if (feature_set == "HalfKAv2") {
        return new FeaturedBatchStream<FeatureSet<HalfKAv2>, SparseBatch>(
            concurrency, filename, batch_size, cyclic, skipPredicate
        );
    }
    if (feature_set == "HalfKAv2^") {
        return new FeaturedBatchStream<FeatureSet<HalfKAv2Factorized>, SparseBatch>(
            concurrency, filename, batch_size, cyclic, skipPredicate
        );
    }
    if (feature_set == "HalfKAv2^+DonorSingleEff") {
        return new FeaturedBatchStream<FeatureSet<HalfKAv2Factorized, DonorSingleEff>, SparseBatch>(
            concurrency, filename, batch_size, cyclic, skipPredicate
        );
    }
    if (feature_set == "HalfKAv2^+DonorPairSlots") {
        return new FeaturedBatchStream<FeatureSet<HalfKAv2Factorized, DonorPairSlots>, SparseBatch>(
            concurrency, filename, batch_size, cyclic, skipPredicate
        );
    }
    if (feature_set == "HalfKAv2^+DonorKnight8Slots") {
        return new FeaturedBatchStream<FeatureSet<HalfKAv2Factorized, DonorKnight8Slots>, SparseBatch>(
            concurrency, filename, batch_size, cyclic, skipPredicate
        );
    }

    fprintf(stderr, "Unknown feature_set %s\n", feature_set_c);
    return nullptr;
}

EXPORT void CDECL destroy_sparse_batch_stream(Stream<SparseBatch>* stream) { delete stream; }

EXPORT SparseBatch* CDECL fetch_next_sparse_batch(Stream<SparseBatch>* stream) {
    return stream->next();
}

EXPORT void CDECL destroy_sparse_batch(SparseBatch* e) { delete e; }

}
