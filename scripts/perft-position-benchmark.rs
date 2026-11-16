// Appended to the unchanged imports + measure() from perft-benchmark.rs.
fn main() {
    use haitaka::extended::compiled as api;
    let args: Vec<String> = std::env::args().collect();
    let depth: u8 = args[2].parse().unwrap();
    let minimum_ms: u64 = args[3].parse().unwrap();
    let samples: usize = args[4].parse().unwrap();
    let input = std::fs::read_to_string(&args[5]).unwrap();
    let mut game = api::CompiledGame::parse(&input).unwrap();
    if samples == 0 {
        println!("{}", serde_json::json!({"nodesPerPerft":game.perft(depth),"depth":depth}));
        return;
    }
    let result = measure(|| black_box(&mut game).perft(black_box(depth)),depth,minimum_ms,samples);
    println!("{}",serde_json::json!({"ruleset":args[1],"position":"fixed-input","bulkCounting":false,"transpositionCache":false,"result":result}));
}
