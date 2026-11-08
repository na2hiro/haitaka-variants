use std::{
    hint::black_box,
    time::{Duration, Instant},
};
fn measure(
    mut run: impl FnMut() -> u64,
    depth: u8,
    minimum_ms: u64,
    samples: usize,
) -> serde_json::Value {
    let mut results = Vec::new();
    let mut expected = Some(black_box(run()));
    for _ in 0..samples {
        let start = Instant::now();
        let mut repetitions = 0u64;
        let mut nodes = 0u64;
        loop {
            let count = black_box(run());
            if let Some(e) = expected {
                assert_eq!(count, e);
            } else {
                expected = Some(count);
            }
            repetitions += 1;
            nodes += count;
            if start.elapsed() >= Duration::from_millis(minimum_ms) {
                break;
            }
        }
        let seconds = start.elapsed().as_secs_f64();
        results.push(serde_json::json!({"repetitions":repetitions,"nodesPerPerft":expected.unwrap(),"totalLeafNodes":nodes,"seconds":seconds,"nps":nodes as f64/seconds}));
    }
    let mut nps: Vec<f64> = results.iter().map(|v| v["nps"].as_f64().unwrap()).collect();
    nps.sort_by(|a, b| a.total_cmp(b));
    serde_json::json!({"depth":depth,"nodesPerPerft":expected.unwrap(),"samples":results,"medianNps":nps[nps.len()/2]})
}
#[cfg(not(feature = "extended"))]
fn perft(board: &haitaka::Board, depth: u8) -> u64 {
    if depth == 0 {
        return 1;
    }
    let mut nodes = 0;
    board.generate_moves(|group| {
        for mv in group {
            let mut next = board.clone();
            next.play_unchecked(mv);
            nodes += perft(&next, depth - 1);
        }
        false
    });
    nodes
}
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rule = &args[1];
    let depth: u8 = args[2].parse().unwrap();
    let minimum_ms: u64 = args[3].parse().unwrap();
    let samples: usize = args[4].parse().unwrap();
    #[cfg(feature = "extended")]
    let result = {
        use haitaka::extended::compiled as api;
        let mut game = api::CompiledGame::parse(&api::start_position_json()).unwrap();
        measure(
            || black_box(&mut game).perft(black_box(depth)),
            depth,
            minimum_ms,
            samples,
        )
    };
    #[cfg(not(feature = "extended"))]
    let result = {
        let board = haitaka::Board::startpos();
        measure(
            || perft(black_box(&board), black_box(depth)),
            depth,
            minimum_ms,
            samples,
        )
    };
    println!(
        "{}",
        serde_json::json!({"ruleset":rule,"position":"initial","bulkCounting":false,"transpositionCache":false,"result":result})
    );
}
