## Donor-Family NNUE Feature Plan

### Completion Status (2026-08-17)

This plan is complete and the design below is retained as implementation
history. Donor feature families are accepted by `haitaka_learn`, exported by
the trainer, loaded by Haitaka, and used by the current variant configs.

The original v1 allowance for full-refresh donor evaluation was temporary. The
runtime now applies donor feature deltas to incremental accumulators. The
current Anhoku configuration uses `HalfKAv2^+DonorSingleEff`; remaining
strength work is tracked in
[Anhoku NNUE Handcrafted-Strength Execution Plan](../anhoku-nnue-handcrafted-strength-plan.md).

### Summary

既存の HalfKAv2^ はそのままベースとして維持し、donor 関係は ^ に押し込まず 追加の real feature block として実装する。
方針は 3 系統に固定する。

- standard / handicap: HalfKAv2^ のまま
- single-donor 系: HalfKAv2^+DonorSingleEff
- two-donor-union 系: HalfKAv2^+DonorPairSlots
- knight-8-donor 系: HalfKAv2^+DonorKnight8Slots

^ の意味は今後も「静的 factorization のみ」に限定する。局面依存の donor 情報は export 時に静的畳み込みできないため、training-only virtual feature にはしない。

### Key Changes

- Feature architecture
    - HalfKAv2^ は既存どおり HalfKAv2 + A の static factorization として維持する。
    - DonorSingleEff を新設する。board-only, unkinged, sparse block とし、active feature は oriented_square x piece_color_relative_to_perspective x effective_piece_type。影響を受けた盤上駒にだけ 1 本追加する。
    - DonorPairSlots を新設する。board-only, unkinged, sparse block とし、active feature は oriented_square x piece_color_relative_to_perspective x slot_id(2) x donor_piece_type。影響を受けた盤上駒ごとに slot ごと 0-2 本追加する。
    - DonorKnight8Slots を新設する。board-only, unkinged, sparse block とし、active feature は oriented_square x piece_color_relative_to_perspective x knight_slot_id(8) x donor_piece_type。影響を受けた盤上駒ごとに slot ごと 0-8 本追加する。
    - donor block はすべて持ち駒には feature を立てない。持ち駒は従来どおり HalfKAv2 のみで表現する。
    - single-donor 系は donor の相対位置を geometry に入れず、effective_piece_type に潰して共有 geometry を保つ。Annan と Anhoku は同じ family に乗せる。
    - two-donor-union 系は slot 依存情報を保持する。Antouzai は slot 順を left, right で固定する。
    - knight-8-donor 系は 8 つの八方桂 donor slot を固定順で持つ。順序は mover 基準の knight offset を時計回りで固定し、trainer と runtime で完全一致させる。
- Trainer / external repo (haitaka-variant-nnue-pytorch)
    - Python feature block に DonorSingleEff, DonorPairSlots, DonorKnight8Slots を追加し、features 文字列として上の 3 つの composite feature set を解決できるようにする。
    - C++ data loader に同名 block の sparse feature 展開を追加する。rule family ごとの donor descriptor を block 内に持ち、packed position から donor active feature を直接生成する。
    - feature_set.py の multi-block hash 計算バグを修正する。新しい composite feature set では single-block 前提をやめ、union feature set の hash を正しく計算する。
    - serializer は multi-block real geometry をそのまま export し、HalfKAv2^ 部分だけ既存どおり coalesce する。donor block は coalesce 対象にしない。
- Haitaka runtime
    - NnueModel::from_bytes と feature extractor に feature-family enum を導入し、既存 HalfKAv2 と 3 つの donor family を hash で識別する。
    - runtime の active feature 生成を family-aware にし、base HalfKAv2 real features の後ろへ donor real block の indices を連結する。
    - MAX_ACTIVE_FEATURES を family ごとに見直す。DonorKnight8Slots 用に 128 固定をやめ、少なくとも base + 8 * max_board_pieces を安全に収めるサイズへ上げる。
    - incremental eval の delta 適用は donor family ではいったん full refresh fallback にしてよい。v1 では correctness 優先、最適化は後続タスクに分離する。
- Haitaka learn / config / rule mapping
    - training.features == "HalfKAv2^" の固定検証をやめ、許可 feature set を family ごとに whitelist する。
    - ruleset から推奨 feature set を自動マップする。annan/anhoku -> HalfKAv2^+DonorSingleEff、antouzai -> HalfKAv2^+DonorPairSlots、将来の安騎 ruleset は HalfKAv2^+DonorKnight8Slots。
    - README と example config を更新し、standard/handicap と donor-family variants の feature set 違いを明記する。
    - variant.py / variant.h overlay は 9x9/10 piece-types/pockets を維持し、rule family の違いは feature set 側で扱う。
- Suggested worker split
    - Worker 1: trainer feature blocks + C++ data loader + composite-hash fix
    - Worker 2: Haitaka runtime hash/family parser + active feature extraction + full-refresh path
    - Worker 3: haitaka_learn config/ruleset mapping + docs + verification fixtures/tests

### Test Plan

- haitaka-variant-nnue-pytorch
    - 新 feature set 名 3 種が parser で解決できること
    - multi-block feature set で hash 計算が動くこと
    - 各 donor block の sparse active feature が crafted positions で期待どおりになること
    - serialize / load roundtrip が 3 family すべてで通ること
- Haitaka runtime
    - 既存 HalfKAv2 net が従来どおり load/evaluate できること
    - 各 new family hash の .nnue を NnueModel::from_bytes が読めること
    - Annan, Anhoku, Antouzai, 安騎の代表局面で donor block active features が期待どおりになること
    - donor family で evaluate_full_refresh が安定すること
    - incremental path を full refresh fallback にした family で search smoke が通ること
- End-to-end
    - haitaka_learn が ruleset ごとの推奨 feature set で generate-data, train, export, verify を通せること
    - Annan / Anhoku が同じ DonorSingleEff geometry を共有しつつ別 ruleset で正しく donor 解釈されること
    - Antouzai が left/right slot 順で安定すること
    - 安騎 family は unit tests で 8 knight slot の順序と donor 抽出だけ先行固定し、実 ruleset 導入時の回帰基盤にすること

### Assumptions

- donor 動的特徴は ^ に入れない。^ は static factorization 専用のままにする。
- donor family ごとに exported runtime geometry は変わる。既存 HalfKAv2 family と .nnue 互換にはしない。
- single-donor family は slot 位置を潰して effective_piece_type のみを持つ。これにより front/back など異なる 1-donor rules を同じ geometry に載せる。
- two-donor-union family は slot 依存を保持する。rule ごとの slot 順は descriptor で固定し、feature geometry 自体は family 内で共有する。
- knight-8-donor family は 8 knight offsets の固定順を仕様として先に決める。実 ruleset 名は将来追加でよいが、geometry はこの順序で固定する。
- The original v1 plan allowed full-refresh fallback while correctness was
  established. The current runtime updates donor-family accumulators
  incrementally, so full refresh is no longer the active implementation path.

### Historical Objective Review (Pre-Implementation)

The review below describes the repository before this plan was implemented.
Statements such as “right now” and the full-refresh caveat are historical, not
the current repository state.

Objectively: for donor-rule variants, this is likely a materially important input change, not a cosmetic one.

Right now the pipeline forces HalfKAv2^ only (haitaka_learn/src/config.rs:317, haitaka_learn/README.md:101), and the runtime extractor feeds only physical piece-square and hand features (haitaka_wasm/src/
nnue.rs:18, haitaka_wasm/src/nnue.rs:683). But in Annan/Anhoku/Antouzai, move power depends on donor relationships and effective movement, not just native piece identity (haitaka/src/variant_rules.rs:153,
haitaka/src/variant_rules.rs:181). So the current NNUE is missing a first-class feature for one of the core rules of the game.

That does not mean the current net is blind. It can still infer some donor effects indirectly from raw placements. But HalfKAv2^ is a poor inductive bias for this: the donor relation is a piece-to-piece
interaction, and today the network has to recover that interaction from summed single-piece embeddings. Your plan adds exactly the missing structured signal (plans/extend-nnue-input-for-donor-rules.md:17).
I would expect:

- standard and handicap: essentially no strength impact, since their feature set stays unchanged.
- annan / anhoku: moderate to large strength upside after retraining.
- antouzai: probably the largest upside, because union-of-two-donors is the hardest case for plain HalfKAv2^ to learn cleanly.
- future knight-8 family: probably necessary rather than optional.

Two caveats keep me from saying “guaranteed huge Elo gain”:

- Your labels are still from shallow handcrafted search (search_depth = 3 in the checked-in variant configs), so teacher quality may cap the benefit.
- The v1 runtime plan falls back to full refresh for donor families, which can reduce NPS and partially eat the eval gain in practical play.

So my judgment is: this is high-leverage for donor variants and likely worth doing, but the net playing-strength gain should be described as “likely meaningful, maybe large,” not “certainly dramatic.” The
only objective proof is an A/B retrain plus self-play with fixed time controls and identical search.
