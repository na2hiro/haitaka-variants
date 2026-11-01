// Diagnostic instrumentation by Haitaka contributors, MIT.
// Compiled only into the external GPLv3 reference engine.
if (sub_command == "p1ref") {
    auto& progress = networks().progress;
    std::int64_t black = 0, white = 0;
    const auto bk = pos.square<KING>(BLACK);
    const auto wk = Inv(pos.square<KING>(WHITE));
    const auto* fb = pos.eval_list()->piece_list_fb();
    const auto* fw = pos.eval_list()->piece_list_fw();
    for (int i = 0; i < PIECE_NUMBER_KING; ++i) {
      black += progress.weights_q16_[bk][fb[i]];
      white += progress.weights_q16_[wk][fw[i]];
    }
    const auto sum = progress.SumQ16(pos);
    const auto score = Eval::compute_eval(pos);
    std::cout << "P1REF " << black << ' ' << white << ' ' << sum << ' '
              << progress.BucketIndex(pos, 8) << ' ' << score << std::endl;
  } else
