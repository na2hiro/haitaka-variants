use haitaka::{extended::compiled as api, hash_probe};
use std::{hint::black_box, time::{Duration, Instant}};
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let depth: u8 = args[2].parse().unwrap();
    let mut game = api::CompiledGame::parse(&api::start_position_json()).unwrap();
    let expected = black_box(game.perft(depth));
    let mut samples = vec![];
    for _ in 0..3 {
        hash_probe::reset();
        let start = Instant::now();
        let mut repetitions = 0u64;
        while start.elapsed() < Duration::from_secs(3) {
            assert_eq!(black_box(&mut game).perft(black_box(depth)), expected);
            repetitions += 1;
        }
        let seconds = start.elapsed().as_secs_f64();
        let (calls, nanos) = hash_probe::stats();
        samples.push(serde_json::json!({
            "nodesPerPerft":expected, "repetitions":repetitions, "seconds":seconds,
            "fullHashCalls":calls, "fullHashCallsPerPerft":calls as f64/repetitions as f64,
            "fullHashNanos":nanos, "fullHashTimePercent":nanos as f64/seconds/1e7
        }));
    }
    let mut api_checks = vec![];
    let mut history = game.clone();
    let mut history_plies = 0;
    for _ in 0..16 {
        let moves = history.moves();
        if moves.is_empty() { break; }
        history.play(moves[moves.len()/2]).unwrap();
        history_plies += 1;
    }
    for (label, json, plies) in [
        ("root", game.to_json(), 0),
        ("history", history.to_json(), history_plies),
    ] {
        hash_probe::reset();
        for _ in 0..32 {
            black_box(api::inspect_json(black_box(&json)).unwrap());
        }
        let (calls, nanos) = hash_probe::stats();
        api_checks.push(serde_json::json!({
            "case":label, "historyPlies":plies, "inspectCalls":32,
            "fullHashCalls":calls, "fullHashCallsPerInspect":calls as f64/32.,
            "fullHashNanos":nanos
        }));
    }
    println!("{}", serde_json::json!({"rule":args[1],"depth":depth,"samples":samples,"apiChecks":api_checks}));
}
