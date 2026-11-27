use super::*;

fn trial_counts(stats: SearchOrderingStats) -> [u64; 4] {
    [
        stats.pvs_probes,
        stats.pvs_researches,
        stats.lmr_reductions,
        stats.lmr_researches,
    ]
}

#[test]
fn trial_counts_follow_the_build_and_are_reported_by_usi() {
    let mut session = UsiSession::default();
    let outputs = session.handle_line("go depth 4");
    let info = &outputs[0];
    assert!(info.starts_with("info depth 4 "), "{outputs:?}");
    for field in [
        "pvsProbes",
        "pvsResearches",
        "lmrReductions",
        "lmrResearches",
    ] {
        let value = info
            .split_whitespace()
            .skip_while(|word| *word != field)
            .nth(1);
        if cfg!(feature = "search-pvs") {
            let count: u64 = value.expect(field).parse().unwrap();
            if !SEARCH_TRIALS_SUPPORTED
                || (!cfg!(feature = "search-lmr") && field.starts_with("lmr"))
            {
                assert_eq!(count, 0, "{field}");
            }
            if STANDARD_SEARCH_RULES
                && (field == "pvsProbes"
                    || (cfg!(feature = "search-lmr") && field == "lmrReductions"))
            {
                assert!(count > 0, "{info}");
            }
        } else {
            assert_eq!(value, None, "{info}");
        }
    }
    let board = &session.board;
    let mut tt = TranspositionTable::default();
    let mut ordering = SearchOrdering::default();
    let mut ctx = handcrafted_context(board, &mut tt, &mut ordering, qsearch_limits());
    if !ctx.pvs_enabled() {
        search_best_move(board, 3, &mut ctx, None, &mut RootProgress::default()).unwrap();
        assert_eq!(trial_counts(ctx.ordering_stats), [0; 4]);
    }
}

#[test]
#[cfg(all(
    feature = "search-pvs",
    not(any(feature = "search-lmr", feature = "donor-category"))
))]
fn pvs_matches_baseline_fixed_depth_scores() {
    if !STANDARD_SEARCH_RULES {
        return;
    }
    let mut fixtures = vec![(haitaka::SFEN_STARTPOS, 5)];
    fixtures.extend(ORDERING_TEST_OPENINGS.map(|sfen| (sfen, 5)));
    fixtures.extend([
        ("9/9/k8/9/4Rr3/9/9/9/4K4 b - 1", 5),
        ("4k4/9/4S4/9/9/9/9/9/4K4 b - 1", 5),
        ("4k4/9/4B4/9/9/9/9/9/4K4 b - 1", 5),
        (DFPN_MATE_SFEN, 3),
        ("k8/9/9/9/9/4r4/9/9/4K4 b G 1", 3),
    ]);
    for (sfen, max_depth) in fixtures {
        let board: Board = sfen.parse().unwrap();
        check_pvs_scores(&PositionHistory::new(board), &[max_depth - 1, max_depth]);
    }
    for (sfen, cycle, verdict, terminal_score) in [
        (
            "4k4/9/9/9/9/9/9/9/4K4 b - 1",
            ["5i4i", "5a4a", "4i5i", "4a5a"],
            HistoryAdjudication::RepetitionDraw,
            0,
        ),
        (
            "4k4/9/5R3/9/9/9/9/9/K8 b - 1",
            ["4c5c", "5a4a", "5c4c", "4a5a"],
            HistoryAdjudication::PerpetualCheckLoss(Color::Black),
            -MATE_SCORE,
        ),
    ] {
        let mut board: Board = sfen.parse().unwrap();
        let mut history = PositionHistory::new(board.clone());
        for text in cycle.repeat(3).into_iter().take(11) {
            board.try_play(text.parse().unwrap()).unwrap();
            history.push(board.clone());
        }
        assert_eq!(history.adjudication(), HistoryAdjudication::Ongoing);
        check_pvs_scores(&history, &[3, 4]);
        board.try_play(cycle[3].parse().unwrap()).unwrap();
        history.push(board.clone());
        assert_eq!(history.adjudication(), verdict);
        let (progress, stats) = fixed_root(&history, 4, qsearch_limits(), false);
        assert_eq!(progress.best_score, Some(terminal_score));
        assert_eq!(progress.completed_moves, 0);
        assert_eq!(trial_counts(stats), [0; 4]);
    }
}

#[cfg(all(
    feature = "search-pvs",
    not(any(feature = "search-lmr", feature = "donor-category"))
))]
fn check_pvs_scores(history: &PositionHistory, depths: &[u8]) {
    for limits in [
        qsearch_limits(),
        QsearchLimits {
            delta_min_qply: u8::MAX,
            ..qsearch_limits()
        },
    ] {
        for &depth in depths {
            let baseline = fixed_root(history, depth, limits, true).0;
            let pvs = fixed_root(history, depth, limits, false).0;
            assert_eq!(
                pvs.best_score,
                baseline.best_score,
                "{} at depth {depth}, delta_min_qply {} (PVS {:?}, baseline {:?})",
                history.current(),
                limits.delta_min_qply,
                pvs.best_move,
                baseline.best_move,
            );
        }
    }
}

#[cfg(all(
    feature = "search-pvs",
    not(any(feature = "search-lmr", feature = "donor-category"))
))]
fn fixed_root(
    history: &PositionHistory,
    depth: u8,
    limits: QsearchLimits,
    disable_pvs: bool,
) -> (RootProgress, SearchOrderingStats) {
    let board = history.current();
    let mut tt = TranspositionTable::default();
    let mut ordering = SearchOrdering::default();
    let mut ctx = handcrafted_context(board, &mut tt, &mut ordering, limits);
    ctx.disable_pvs = disable_pvs;
    ctx.history = history.clone();
    let mut progress = RootProgress::default();
    search_best_move(board, depth, &mut ctx, None, &mut progress).unwrap();
    assert!(!ctx.qsearch_soft_cap_reached, "{} depth {depth}", board);
    assert_eq!(ctx.history.positions(), history.positions());
    (progress, ctx.ordering_stats)
}

#[test]
#[cfg(all(
    feature = "annan",
    feature = "search-pvs",
    feature = "search-variant-experiments"
))]
fn annan_pvs_matches_full_window_without_delta_pruning() {
    let board = Board::startpos();
    for delta_min_qply in [DEFAULT_QSEARCH_LIMITS.delta_min_qply, u8::MAX] {
        let mut scores = Vec::new();
        for disable_pvs in [true, false] {
            let mut tt = TranspositionTable::default();
            let mut ordering = SearchOrdering::default();
            let limits = QsearchLimits {
                delta_min_qply,
                ..qsearch_limits()
            };
            let mut ctx = handcrafted_context(&board, &mut tt, &mut ordering, limits);
            ctx.disable_pvs = disable_pvs;
            ctx.disable_lmr = true;
            let mut progress = RootProgress::default();
            search_best_move(&board, 4, &mut ctx, None, &mut progress).unwrap();
            assert!(!ctx.qsearch_soft_cap_reached);
            scores.push(progress.best_score);
        }
        eprintln!("Annan startpos depth 4, delta_min_qply {delta_min_qply}: {scores:?}");
        if delta_min_qply == u8::MAX {
            assert_eq!(scores[0], scores[1]);
        }
    }
}

#[test]
#[cfg(feature = "search-pvs")]
fn pvs_research_interruption_keeps_the_first_completed_root_move() {
    if !STANDARD_SEARCH_RULES {
        return;
    }
    let board: Board = haitaka::SFEN_STARTPOS.parse().unwrap();
    let mut tt = TranspositionTable::default();
    let mut ordering = SearchOrdering::default();
    let mut ctx = handcrafted_context(&board, &mut tt, &mut ordering, qsearch_limits());

    let mut picker = MovePicker::new(&board, None, ctx.ordering, 0);
    let first = picker.next().unwrap().mv;
    let second = picker.next().unwrap().mv;
    let child_side = !board.side_to_move();
    for (mv, child_score) in [(first, 40), (second, -10)] {
        let mut child = board.clone();
        child.play_unchecked(mv);
        ctx.history.push(child);
        let key = ctx.history.tt_key();
        ctx.history.pop();
        let probe = ctx.tt.probe(key, child_side);
        ctx.tt
            .write(probe, key, child_score, false, Bound::Exact, 1, None, 0);
    }
    // Count the root, the first child, and the second child's improving probe.
    // The second move's full-window replay then exhausts the node budget.
    ctx.node_budget = Some(Arc::new(SharedNodeBudget::new(3)));
    let mut progress = RootProgress::default();
    let status = search_best_move(&board, 2, &mut ctx, None, &mut progress);
    assert_eq!(
        status,
        Err(SearchInterrupted::Stopped(
            SearchInterruptionReason::NodeBudget
        ))
    );
    assert_eq!(trial_counts(ctx.ordering_stats), [1, 1, 0, 0]);
    assert_eq!(progress.completed_moves, 1);
    assert_eq!(progress.best_move, Some(first));
    assert_eq!(progress.best_score, Some(-40));
    assert_eq!(ctx.history.positions(), std::slice::from_ref(&board));
    assert!(!ctx.in_trial);
}

#[test]
#[cfg(feature = "search-pvs")]
fn qsearch_cap_replays_the_root_child_and_disables_trials_and_tt_stores() {
    if !STANDARD_SEARCH_RULES {
        return;
    }
    let board: Board = haitaka::SFEN_STARTPOS.parse().unwrap();
    let (first, first_qnodes, first_stores) = stop_after_first_child(&board);
    let limits = QsearchLimits {
        node_limit: first_qnodes,
        ..qsearch_limits()
    };
    for interrupt in [false, true] {
        let mut tt = TranspositionTable::default();
        let mut ordering = SearchOrdering::default();
        let mut ctx = handcrafted_context(&board, &mut tt, &mut ordering, limits);
        ctx.training_trace = Some(TrainingTraceCollector::default());
        if interrupt {
            // The probe reaches the cap on its first qnode. Interrupt on
            // the next qnode, during the full-window replay.
            ctx.forced_interruption = Some(ForcedInterruption::AfterQnode(first_qnodes + 2));
        }

        let mut result = RootProgress::default();
        let status = search_best_move(&board, 2, &mut ctx, None, &mut result);
        assert!(ctx.qsearch_soft_cap_reached);
        assert!(!ctx.in_trial);
        assert_eq!(trial_counts(ctx.ordering_stats), [1, 1, 0, 0]);
        assert_eq!(ctx.tt_stats.tt_stores, first_stores);
        assert_eq!(ctx.history.positions(), std::slice::from_ref(&board));

        if interrupt {
            assert_eq!(
                status,
                Err(SearchInterrupted::Stopped(
                    SearchInterruptionReason::ForcedInsideQsearch
                ))
            );
            assert_eq!(result.completed_moves, 1);
            assert_eq!(result.best_move, first.best_move);
            assert_eq!(result.best_score, first.best_score);
            assert_eq!(result.training_trace, first.training_trace);
        } else {
            status.unwrap();
            assert_eq!(result.completed_moves as usize, count_legal_moves(&board));
            // The first child was uncapped in both runs. Every remaining
            // child should match the baseline's ordinary window after the cap.
            let mut baseline_tt = TranspositionTable::default();
            let mut baseline_ordering = SearchOrdering::default();
            let mut baseline =
                handcrafted_context(&board, &mut baseline_tt, &mut baseline_ordering, limits);
            baseline.disable_pvs = true;
            let mut expected = RootProgress::default();
            search_best_move(&board, 2, &mut baseline, None, &mut expected).unwrap();
            assert_eq!(result.best_score, expected.best_score);
            assert_eq!(result.best_move, expected.best_move);
        }
    }
}

#[cfg(feature = "search-pvs")]
fn stop_after_first_child(board: &Board) -> (RootProgress, u64, u64) {
    let mut tt = TranspositionTable::default();
    let mut ordering = SearchOrdering::default();
    let mut ctx = handcrafted_context(board, &mut tt, &mut ordering, qsearch_limits());
    ctx.training_trace = Some(TrainingTraceCollector::default());
    ctx.forced_interruption = Some(ForcedInterruption::AfterRootChild(1));
    let mut progress = RootProgress::default();
    let stopped = search_best_move(board, 2, &mut ctx, None, &mut progress).unwrap_err();
    assert_eq!(
        stopped.reason(),
        SearchInterruptionReason::ForcedAfterRootChild
    );
    assert!(progress.training_trace.is_some());
    (progress, ctx.qsearch_stats.qnodes, ctx.tt_stats.tt_stores)
}

#[test]
#[cfg(feature = "search-lmr")]
fn lmr_fail_low_saves_the_reduced_child_at_one_ply_less_and_skips_the_parent() {
    if !STANDARD_SEARCH_RULES {
        return;
    }
    let board: Board = haitaka::SFEN_STARTPOS.parse().unwrap();
    let mut tt = TranspositionTable::default();
    let mut ordering = SearchOrdering::default();
    let mut picker = MovePicker::new(&board, None, &ordering, 0);
    let mut child_keys = Vec::new();
    let mut history = PositionHistory::new(board.clone());
    for index in 0..5 {
        let mv = picker.next().unwrap().mv;
        let mut child = board.clone();
        child.play_unchecked(mv);
        history.push(child);
        if index >= 3 {
            child_keys.push(history.tt_key());
        }
        history.pop();
    }

    let mut ctx = handcrafted_context(&board, &mut tt, &mut ordering, qsearch_limits());
    assert_eq!(
        negamax(&board, 3, 250, 251, 0, &mut ctx, None).unwrap(),
        250
    );
    assert_eq!(
        ctx.ordering_stats.lmr_reductions,
        (count_legal_moves(&board) - 4) as u64
    );
    assert_eq!(ctx.ordering_stats.lmr_researches, 0);
    assert_eq!(ctx.history.positions(), std::slice::from_ref(&board));
    let child_side = !board.side_to_move();
    assert_eq!(
        ctx.tt.probe(child_keys[0], child_side).data.unwrap().depth,
        2
    );
    assert_eq!(
        ctx.tt.probe(child_keys[1], child_side).data.unwrap().depth,
        1
    );
    assert!(
        ctx.tt
            .probe(history.tt_key(), board.side_to_move())
            .data
            .is_none()
    );
}

#[test]
#[cfg(feature = "search-lmr")]
fn lmr_fail_high_or_mate_researches_with_the_full_child_depth() {
    if !STANDARD_SEARCH_RULES {
        return;
    }
    let board: Board = haitaka::SFEN_STARTPOS.parse().unwrap();
    for (alpha, reduced_mate) in [(-5_000, false), (250, true)] {
        let mut tt = TranspositionTable::default();
        let mut ordering = SearchOrdering::default();
        let mut ctx = handcrafted_context(&board, &mut tt, &mut ordering, qsearch_limits());
        let key = ctx.history.tt_key();
        let side = board.side_to_move();
        if reduced_mate {
            // Only the reduced search can accept this depth-1 mate entry.
            // From the parent it is a fail-low inside the mate band.
            let probe = ctx.tt.probe(key, side);
            ctx.tt.write(
                probe,
                key,
                score_to_tt(MATE_SCORE - 2, 1),
                false,
                Bound::Exact,
                1,
                None,
                0,
            );
        }
        let score = search_main_child(
            &board,
            2,
            (alpha, alpha + 1),
            1,
            &mut ctx,
            None,
            ChildTrial::Lmr,
        )
        .unwrap()
        .expect("the trial should trigger a full-depth replay");
        if reduced_mate {
            assert_eq!(ctx.tt_stats.tt_cutoffs, 1);
            assert!(!is_mate_score(score));
        } else {
            assert!(score > alpha);
        }
        assert_eq!(trial_counts(ctx.ordering_stats), [0, 0, 1, 1]);
        assert_eq!(ctx.tt.probe(key, side).data.unwrap().depth, 2);
    }
}

#[test]
#[cfg(feature = "search-lmr")]
fn lmr_protects_wide_shallow_checked_and_mate_windows() {
    if !STANDARD_SEARCH_RULES {
        return;
    }
    let board: Board = haitaka::SFEN_STARTPOS.parse().unwrap();
    let checked: Board = "k8/9/9/9/9/4r4/9/9/4K4 b G 1".parse().unwrap();
    assert!(!checked.checkers().is_empty());
    for (board, depth, alpha, beta) in [
        (&board, 3, 250, 252),
        (&board, 2, 250, 251),
        (&board, 3, MATE_TT_THRESHOLD - 1, MATE_TT_THRESHOLD),
        (&board, 3, MATE_TT_THRESHOLD, MATE_TT_THRESHOLD + 1),
        (&checked, 3, 250, 251),
    ] {
        let mut tt = TranspositionTable::default();
        let mut ordering = SearchOrdering::default();
        let mut ctx = handcrafted_context(board, &mut tt, &mut ordering, qsearch_limits());
        negamax(board, depth, alpha, beta, 0, &mut ctx, None).unwrap();
        assert_eq!(
            ctx.ordering_stats.lmr_reductions, 0,
            "{board} {depth} {alpha} {beta}"
        );
    }
}

#[test]
#[cfg(feature = "search-lmr")]
fn lmr_protects_hash_killers_captures_promotions_drops_and_checks() {
    if !STANDARD_SEARCH_RULES {
        return;
    }
    let board: Board = haitaka::SFEN_STARTPOS.parse().unwrap();
    let tactical: Board = "4k4/9/5S3/5p3/9/5R3/9/9/4K4 b G 1".parse().unwrap();
    for (board, text, source, eligible) in [
        (&board, "7g7f", MoveSource::History, true),
        (&board, "7g7f", MoveSource::Hash, false),
        (&board, "7g7f", MoveSource::Killer, false),
        (&board, "7g7f", MoveSource::Tactical, false),
        (&tactical, "4f3f", MoveSource::History, true),
        (&tactical, "4f4d", MoveSource::History, false),
        (&tactical, "4c3b+", MoveSource::History, false),
        (&tactical, "G*7g", MoveSource::History, false),
        (&tactical, "4f5f", MoveSource::History, false),
    ] {
        let mv = text.parse().unwrap();
        let mut child = board.clone();
        child.try_play(mv).unwrap();
        assert_eq!(
            lmr_quiet_move(board, &child, PickedMove { mv, source }),
            eligible,
            "{text}"
        );
    }
}

#[test]
#[cfg(feature = "search-lmr")]
fn lmr_protects_a_mate_score_already_seen_at_the_node() {
    if !STANDARD_SEARCH_RULES {
        return;
    }
    let board: Board = haitaka::SFEN_STARTPOS.parse().unwrap();
    let mut tt = TranspositionTable::default();
    let mut ordering = SearchOrdering::default();
    let mut ctx = handcrafted_context(&board, &mut tt, &mut ordering, qsearch_limits());
    let child_side = !board.side_to_move();
    // Give every legal child a proven winning mate, so the first full search
    // establishes a mate loss while the parent window remains outside the mate band.
    let mate_score = MATE_SCORE - 2;
    let mut picker = MovePicker::new(&board, None, ctx.ordering, 0);
    while let Some(picked) = picker.next() {
        let mut child = board.clone();
        child.play_unchecked(picked.mv);
        ctx.history.push(child);
        let key = ctx.history.tt_key();
        ctx.history.pop();
        let probe = ctx.tt.probe(key, child_side);
        ctx.tt.write(
            probe,
            key,
            score_to_tt(mate_score, 1),
            false,
            Bound::Exact,
            2,
            None,
            0,
        );
    }
    assert_eq!(
        negamax(&board, 3, 250, 251, 0, &mut ctx, None).unwrap(),
        -mate_score
    );
    assert_eq!(ctx.tt_stats.tt_cutoffs, count_legal_moves(&board) as u64);
    assert_eq!(ctx.ordering_stats.lmr_reductions, 0);
}
