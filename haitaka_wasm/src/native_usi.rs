use std::sync::atomic::AtomicBool;

use super::{Color, Instant, SearchControl, SearchSummary, UsiSearchBudget, UsiSession, tt};

pub const MAX_DEPTH: u8 = 100;

pub struct NativeUsiSession {
    session: UsiSession,
    default_depth: u8,
    previous_depth: Option<u8>,
    position_valid: bool,
}

impl NativeUsiSession {
    pub fn new(engine_name: String, max_depth: u8) -> Self {
        Self {
            session: UsiSession::new(engine_name, max_depth),
            default_depth: max_depth,
            previous_depth: None,
            position_valid: true,
        }
    }

    pub fn load_nnue(&mut self, bytes: &[u8], multiplier: i32) -> Result<String, String> {
        self.session.load_nnue_with_multiplier(bytes, multiplier)
    }

    /// Handle a non-search command while no search owns the session.
    pub fn handle_line(&mut self, line: &str) -> Vec<String> {
        let words = line.split_whitespace().collect::<Vec<_>>();
        match words.as_slice() {
            ["usi"] => {
                let mut response = self.session.handle_line(line);
                let options_start = 1;
                response.splice(
                    options_start..=options_start,
                    [
                        "id author Haitaka Variants contributors".to_string(),
                        format!(
                            "option name USI_Hash type spin default {} min {} max {}",
                            tt::DEFAULT_HASH_MB,
                            tt::MIN_HASH_MB,
                            tt::MAX_HASH_MB
                        ),
                        format!(
                            "option name MaxDepth type spin default {} min 1 max {MAX_DEPTH}",
                            self.default_depth
                        ),
                    ],
                );
                response
            }
            ["setoption", "name", "USI_Hash", "value", value] => self
                .session
                .handle_line(&format!("setoption name Hash value {value}")),
            ["setoption", "name", "USI_Ponder", "value", "false" | "true"] => Vec::new(),
            ["setoption", "name", "MaxDepth", "value", value] => match value.parse::<u8>() {
                Ok(depth @ 1..=MAX_DEPTH) => {
                    self.session.movetime_max_depth = depth;
                    Vec::new()
                }
                _ => vec![format!(
                    "info string MaxDepth must be between 1 and {MAX_DEPTH}"
                )],
            },
            ["position", ..] => {
                let response = self.session.handle_line(line);
                self.position_valid = response.is_empty();
                response
            }
            ["usinewgame"] | ["gameover", "win" | "lose" | "draw"] => {
                self.session
                    .apply_position("position startpos")
                    .expect("compiled startpos should be legal");
                self.session.tt.clear();
                self.previous_depth = None;
                self.position_valid = true;
                Vec::new()
            }
            _ => self.session.handle_line(line),
        }
    }

    pub fn prepare_search(&self, line: &str) -> Result<NativeGo, String> {
        if !self.position_valid {
            return Err("no valid position".to_string());
        }
        parse_go(
            line,
            self.session.board.side_to_move(),
            self.session.movetime_max_depth,
        )
    }

    pub fn search(
        &mut self,
        request: NativeGo,
        stop: &AtomicBool,
        send_info: impl Fn(String),
    ) -> Vec<String> {
        let cap = match request.budget {
            UsiSearchBudget::Depth(depth) => depth,
            UsiSearchBudget::Movetime { max_depth, .. } => max_depth,
            UsiSearchBudget::Nodes(_) => super::SEARCH_NODE_BUDGET_MAX_DEPTH,
        };
        if self
            .previous_depth
            .replace(cap)
            .is_some_and(|prev| cap < prev)
        {
            self.session.tt.clear();
        }
        let progress = |depth: u8, summary: &SearchSummary| {
            let elapsed_ms = summary.elapsed_ms.max(0.0);
            let mut line = format!(
                "info depth {depth} time {elapsed_ms:.0} nodes {} nps {:.0}",
                summary.states, summary.nps
            );
            if let Some(best_move) = &summary.best_move {
                line.push_str(" pv ");
                line.push_str(best_move);
            }
            send_info(line);
        };
        self.session.search_outputs_for_budget(
            request.budget,
            SearchControl {
                deadline: request.expired_clock.then(Instant::now),
                stop: Some(stop),
                on_iteration: request.infinite.then_some(&progress),
            },
        )
    }
}

pub struct NativeGo {
    budget: UsiSearchBudget,
    expired_clock: bool,
    pub infinite: bool,
}

fn parse_go(line: &str, side: Color, max_depth: u8) -> Result<NativeGo, String> {
    let mut tokens = line.split_whitespace().skip(1);
    let mut depth = None;
    let mut movetime = None;
    let mut nodes = None;
    let mut btime = None;
    let mut wtime = None;
    let mut byoyomi = None;
    let mut binc = None;
    let mut winc = None;
    let mut infinite = false;
    while let Some(key) = tokens.next() {
        let target = match key {
            "depth" => &mut depth,
            "movetime" => &mut movetime,
            "nodes" => &mut nodes,
            "btime" => &mut btime,
            "wtime" => &mut wtime,
            "byoyomi" => &mut byoyomi,
            "binc" => &mut binc,
            "winc" => &mut winc,
            "infinite" if !infinite => {
                infinite = true;
                continue;
            }
            _ => return Err(format!("unexpected go parameter {key}")),
        };
        let value = tokens
            .next()
            .ok_or_else(|| format!("missing {key} value"))?;
        let parsed = value
            .parse::<u64>()
            .map_err(|_| format!("invalid {key} value"))?;
        if target.replace(parsed).is_some() {
            return Err(format!("duplicate {key}"));
        }
    }

    let has_clock = [btime, wtime, byoyomi, binc, winc]
        .iter()
        .any(Option::is_some);
    if nodes.is_some() && (infinite || has_clock || movetime.is_some() || depth.is_some()) {
        return Err("go nodes is mutually exclusive with other limits".to_string());
    }
    if (movetime.is_some() && has_clock) || (infinite && (has_clock || movetime.is_some())) {
        return Err("conflicting go time controls".to_string());
    }
    let depth = depth
        .map(|value| u8::try_from(value).map(|depth| depth.max(1)))
        .transpose()
        .map_err(|_| "invalid depth value".to_string())?;
    let mut expired_clock = false;
    let budget = if let Some(nodes) = nodes {
        if nodes == 0 {
            return Err("nodes value must be greater than 0".to_string());
        }
        UsiSearchBudget::Nodes(nodes)
    } else if let Some(millis) = movetime {
        UsiSearchBudget::Movetime {
            max_depth: depth.unwrap_or(max_depth),
            millis: u32::try_from(millis).map_err(|_| "invalid movetime value".to_string())?,
        }
    } else if has_clock {
        let btime = btime.ok_or("missing btime value")?;
        let wtime = wtime.ok_or("missing wtime value")?;
        if byoyomi.is_some() && (binc.is_some() || winc.is_some()) {
            return Err("byoyomi and increments are mutually exclusive".to_string());
        }
        let (remaining, increment) = if side == Color::Black {
            (btime, binc)
        } else {
            (wtime, winc)
        };
        let millis = clock_budget_ms(remaining, byoyomi.or(increment).unwrap_or(0));
        expired_clock = millis == 0;
        UsiSearchBudget::Movetime {
            max_depth: depth.unwrap_or(max_depth),
            millis,
        }
    } else if infinite {
        UsiSearchBudget::Movetime {
            max_depth: depth.unwrap_or(max_depth),
            millis: 0,
        }
    } else if let Some(depth) = depth {
        UsiSearchBudget::Depth(depth)
    } else {
        UsiSearchBudget::Movetime {
            max_depth,
            millis: 1000,
        }
    };
    Ok(NativeGo {
        budget,
        expired_clock,
        infinite,
    })
}

fn clock_budget_ms(remaining_ms: u64, extra_ms: u64) -> u32 {
    // Spread main time across approximately 30 moves. Keep a small margin
    // for delivery of bestmove, including when only byoyomi remains.
    let target = (remaining_ms / 30).saturating_add(extra_ms);
    let margin = (target / 10).clamp(1, 50);
    u32::try_from(target.saturating_sub(margin)).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clocks_budget_the_moving_side_and_distinguish_zero_from_unlimited() {
        let clocks = "go btime 60000 wtime 30000 binc 500 winc 250";
        let black = parse_go(clocks, Color::Black, 8).unwrap();
        let white = parse_go(clocks, Color::White, 8).unwrap();
        assert_eq!(
            black.budget,
            UsiSearchBudget::Movetime {
                max_depth: 8,
                millis: 2450
            }
        );
        assert_eq!(
            white.budget,
            UsiSearchBudget::Movetime {
                max_depth: 8,
                millis: 1200
            }
        );
        let byoyomi = parse_go("go btime 0 wtime 0 byoyomi 100", Color::Black, 8).unwrap();
        assert_eq!(
            byoyomi.budget,
            UsiSearchBudget::Movetime {
                max_depth: 8,
                millis: 90
            }
        );
        let zero = parse_go("go btime 0 wtime 0 byoyomi 0", Color::Black, 8).unwrap();
        assert!(zero.expired_clock);
        assert!(
            !parse_go("go movetime 0 depth 1", Color::Black, 8)
                .unwrap()
                .expired_clock
        );
        assert!(parse_go("go infinite", Color::Black, 8).unwrap().infinite);
        assert!(parse_go("go nodes 12 depth 2", Color::Black, 8).is_err());
    }
}
