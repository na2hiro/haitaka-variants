use std::time::Instant;
fn main() {
    use haitaka::extended::compiled as api;
    let args: Vec<String> = std::env::args().collect();
    let walks: u32 = args[1].parse().unwrap();
    let first: u32 = args.get(3).map_or(0, |a| a.parse().unwrap());
    let capture_bias: bool = args.get(4).is_none_or(|a| a == "1");
    let mut digest = 0xcbf29ce484222325u64;
    let plies: usize = args[2].parse().unwrap();
    let t0 = Instant::now();
    let mut total_moves = 0u64;
    let mut slowest = (0.0f64, 0u32, 0usize);
    for walk in first..first + walks {
        let mut g = api::CompiledGame::parse(&api::start_position_json()).unwrap();
        let mut seed = 0x5eed_0001_u32.wrapping_add(api::RULE_ID.wrapping_mul(977)).wrapping_add(walk * 7919);
        for ply in 0..plies {
            let t = Instant::now();
            let moves = g.moves();
            let outcome = g.outcome();
            for b in format!("{:?}{:?}", moves, outcome).bytes() { digest = (digest ^ b as u64).wrapping_mul(0x100000001b3); }
            let dt = t.elapsed().as_secs_f64();
            if dt > slowest.0 { slowest = (dt, walk, ply); }
            total_moves += moves.len() as u64;
            if moves.is_empty() { break; }
            seed ^= seed << 13; seed ^= seed >> 17; seed ^= seed << 5;
            let wire = g.current().to_wire();
            let occupied = |at: [u16; 2]| wire.pieces.iter().any(|q| q.at == at);
            let captures: Vec<_> = moves.iter().copied().filter(|m| matches!(m, haitaka::extended::classical::ClassicalMove::Move { path, .. } if occupied(path[0]))).collect();
            let pool = if capture_bias && !captures.is_empty() && seed % 3 == 0 { &captures } else { &moves };
            if std::env::var("TRACE").is_ok() { eprintln!("walk {walk} ply {ply} {:.3}s", dt); }
            g.play(pool[(seed >> 2) as usize % pool.len()]).unwrap();
        }
    }
    println!("{{\"digest\":\"{:016x}\",\"seconds\":{:.3},\"moves\":{},\"slowestSec\":{:.4},\"slowestWalk\":{},\"slowestPly\":{}}}", digest, t0.elapsed().as_secs_f64(), total_moves, slowest.0, slowest.1, slowest.2);
}
