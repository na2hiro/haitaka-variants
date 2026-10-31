use super::*;
use crate::sfnn::MICRO_ARCH;

#[test]
#[ignore = "requires micro; explicit P2 runtime verification"]
fn sfnn_runtime_contract() {
    let dir = std::path::PathBuf::from(
        std::env::var_os("HAITAKA_SFNN_MODEL_DIR").expect("model directory required"),
    );
    let nn = std::fs::read(dir.join("nn.bin")).unwrap();
    let progress = std::fs::read(dir.join("progress.bin")).unwrap();
    fn seed(tt: &mut TranspositionTable, key: u64) {
        let probe = tt.probe(key, Color::Black);
        tt.write(probe, key, 123, false, Bound::Exact, 2, None, 123);
        assert!(tt.probe(key, Color::Black).found);
    }
    assert_eq!(
        load_sfnn_impl(&nn, &progress, MICRO_ARCH).unwrap(),
        MICRO_ARCH
    );
    let global = current_evaluation_strategy();
    let EvaluationStrategy::Sfnn {
        model: global_model,
        ..
    } = &global
    else {
        panic!()
    };
    seed(&mut search_tt_slot().write().unwrap(), 42);
    assert!(
        load_sfnn_impl(&nn, &progress[..8], MICRO_ARCH)
            .unwrap_err()
            .starts_with("SFNN_INVALID_PROGRESS:")
    );
    assert!(load_nnue_impl(&[0; 16]).is_err());
    assert!(
        search_tt_slot()
            .write()
            .unwrap()
            .probe(42, Color::Black)
            .found
    );
    let EvaluationStrategy::Sfnn { model, .. } = current_evaluation_strategy() else {
        panic!()
    };
    assert!(Arc::ptr_eq(global_model, &model));

    let mut session = UsiSession::default();
    seed(&mut session.tt, 99);
    assert!(session.load_sfnn(&nn[..12], &progress, MICRO_ARCH).is_err());
    assert!(matches!(
        session.evaluation,
        EvaluationStrategy::Handcrafted
    ));
    assert!(session.tt.probe(99, Color::Black).found);
    assert_eq!(
        session.load_sfnn(&nn, &progress, MICRO_ARCH).unwrap(),
        MICRO_ARCH
    );
    assert!(!session.tt.probe(99, Color::Black).found);
    let EvaluationStrategy::Sfnn {
        model: session_model,
        ..
    } = session.evaluation_strategy()
    else {
        panic!()
    };
    assert!(!Arc::ptr_eq(global_model, &session_model));
    assert!(
        search_tt_slot()
            .write()
            .unwrap()
            .probe(42, Color::Black)
            .found
    );
    seed(&mut session.tt, 100);
    assert!(session.load_sfnn(&nn, &[], MICRO_ARCH).is_err());
    assert!(session.tt.probe(100, Color::Black).found);
    assert!(session.load_nnue(&[0; 16]).is_err());
    let EvaluationStrategy::Sfnn { model, .. } = session.evaluation_strategy() else {
        panic!()
    };
    assert!(Arc::ptr_eq(&model, &session_model));
    assert!(matches!(
        UsiSession::default().evaluation,
        EvaluationStrategy::Handcrafted
    ));

    // A successful global replacement flushes only the global table.
    assert_eq!(
        load_sfnn_impl(&nn, &progress, MICRO_ARCH).unwrap(),
        MICRO_ARCH
    );
    assert!(
        !search_tt_slot()
            .write()
            .unwrap()
            .probe(42, Color::Black)
            .found
    );
    assert!(session.tt.probe(100, Color::Black).found);
    let EvaluationStrategy::Sfnn { model, .. } = current_evaluation_strategy() else {
        panic!()
    };
    assert!(!Arc::ptr_eq(global_model, &model));
    assert!(Arc::ptr_eq(
        &session_model,
        match &session.evaluation {
            EvaluationStrategy::Sfnn { model, .. } => model,
            _ => panic!(),
        }
    ));
    let output = session.handle_line("go depth 1");
    let mv = output
        .last()
        .unwrap()
        .strip_prefix("bestmove ")
        .unwrap()
        .parse()
        .unwrap();
    assert!(session.board.is_legal(mv));
    *evaluation_slot().write().unwrap() = EvaluationStrategy::Handcrafted;
    search_tt_slot().write().unwrap().clear();
}

#[test]
#[ignore = "requires both micro and mini; explicit P6 profile-switch verification"]
fn sfnn_profile_switches_clear_tt_and_return_the_selected_arch() {
    let micro = std::path::PathBuf::from(
        std::env::var_os("HAITAKA_SFNN_MODEL_DIR").expect("micro directory required"),
    );
    let mini = std::path::PathBuf::from(
        std::env::var_os("HAITAKA_SFNN_MINI_MODEL_DIR").expect("mini directory required"),
    );
    let mut session = UsiSession::default();
    for (dir, arch) in [
        (&micro, crate::sfnn::MICRO_ARCH),
        (&mini, crate::sfnn::MINI_ARCH),
        (&micro, crate::sfnn::MICRO_ARCH),
    ] {
        let nn = std::fs::read(dir.join("nn.bin")).unwrap();
        let progress = std::fs::read(dir.join("progress.bin")).unwrap();
        let probe = session.tt.probe(42, Color::Black);
        session
            .tt
            .write(probe, 42, 123, false, Bound::Exact, 2, None, 123);
        assert!(session.tt.probe(42, Color::Black).found);
        assert_eq!(session.load_sfnn(&nn, &progress, arch).unwrap(), arch);
        assert!(!session.tt.probe(42, Color::Black).found);
        assert_eq!(load_sfnn_impl(&nn, &progress, arch).unwrap(), arch);
        let result = session.handle_line("go depth 1");
        let mv = result
            .last()
            .unwrap()
            .strip_prefix("bestmove ")
            .unwrap()
            .parse()
            .unwrap();
        assert!(session.board.is_legal(mv));
    }
    *evaluation_slot().write().unwrap() = EvaluationStrategy::Handcrafted;
    search_tt_slot().write().unwrap().clear();
}
