//! Strict, search-free legality and terminal replay for Step 14 browser games.
//!
//! This binary deliberately has no evaluator, model, search, browser, or
//! training path. It replays frozen USI moves with the bound Anhoku
//! `Board`/`PositionHistory` APIs and emits one explicit receipt per input row.

use anyhow::{anyhow, bail, ensure, Context, Result};
use clap::Parser;
use haitaka::{Board, Color, GameStatus, HistoryAdjudication, Move, Piece, PositionHistory};
use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{json, Map, Number, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fmt;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::str::FromStr;

const INPUT_SCHEMA: &str = "haitaka-anhoku-strength-first-step14-replay-input-v1";
const OUTPUT_SCHEMA: &str = "haitaka-anhoku-strength-first-step14-replay-v1";
const MAX_PLIES: usize = 200;

#[derive(Debug, Parser)]
#[command(name = "strength-first-step14-replay")]
#[command(about = "Strict search-free Step 14 Anhoku game replay")]
struct Args {
    /// Strict JSONL input. The output is replaced, never appended.
    #[arg(long)]
    input: PathBuf,
    /// Step 14 replay-v1 JSONL receipt.
    #[arg(long)]
    output: PathBuf,
    /// Maximum played plies for this contract (200 production, 2 rehearsal).
    #[arg(long, default_value_t = MAX_PLIES)]
    max_plies: usize,
}

/// A recursively duplicate-key-rejecting JSON value.
struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct StrictVisitor;

        impl<'de> Visitor<'de> for StrictVisitor {
            type Value = Value;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON value without duplicate object keys")
            }

            fn visit_bool<E>(self, value: bool) -> std::result::Result<Value, E> {
                Ok(Value::Bool(value))
            }

            fn visit_i64<E>(self, value: i64) -> std::result::Result<Value, E> {
                Ok(Value::Number(Number::from(value)))
            }

            fn visit_u64<E>(self, value: u64) -> std::result::Result<Value, E> {
                Ok(Value::Number(Number::from(value)))
            }

            fn visit_f64<E>(self, value: f64) -> std::result::Result<Value, E>
            where
                E: serde::de::Error,
            {
                Number::from_f64(value)
                    .map(Value::Number)
                    .ok_or_else(|| E::custom("non-finite JSON number"))
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Value::String(value.to_owned()))
            }

            fn visit_string<E>(self, value: String) -> std::result::Result<Value, E> {
                Ok(Value::String(value))
            }

            fn visit_none<E>(self) -> std::result::Result<Value, E> {
                Ok(Value::Null)
            }

            fn visit_unit<E>(self) -> std::result::Result<Value, E> {
                Ok(Value::Null)
            }

            fn visit_some<D>(self, deserializer: D) -> std::result::Result<Value, D::Error>
            where
                D: Deserializer<'de>,
            {
                StrictValue::deserialize(deserializer).map(|value| value.0)
            }

            fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element::<StrictValue>()? {
                    values.push(value.0);
                }
                Ok(Value::Array(values))
            }

            fn visit_map<A>(self, mut object: A) -> std::result::Result<Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut values = Map::new();
                while let Some(key) = object.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(serde::de::Error::custom(format!(
                            "duplicate JSON key {key:?}"
                        )));
                    }
                    let value = object.next_value::<StrictValue>()?;
                    values.insert(key, value.0);
                }
                Ok(Value::Object(values))
            }
        }

        deserializer.deserialize_any(StrictVisitor).map(StrictValue)
    }
}

#[derive(Clone, Default)]
struct Checks {
    schema: bool,
    identity: bool,
    unique_game_identity: bool,
    sfen: bool,
    side_to_move: bool,
    seats: bool,
    plies: bool,
    move_syntax: bool,
    move_legality: bool,
    no_move_after_terminal: bool,
    history: bool,
    terminal_precedence: bool,
    cap_semantics: bool,
    disposition: bool,
    winner: bool,
}

impl Checks {
    fn value(&self) -> Value {
        json!({
            "schema": self.schema,
            "identity": self.identity,
            "uniqueGameIdentity": self.unique_game_identity,
            "sfen": self.sfen,
            "sideToMove": self.side_to_move,
            "seats": self.seats,
            "plies": self.plies,
            "moveSyntax": self.move_syntax,
            "moveLegality": self.move_legality,
            "noMoveAfterTerminal": self.no_move_after_terminal,
            "history": self.history,
            "terminalPrecedence": self.terminal_precedence,
            "capSemantics": self.cap_semantics,
            "disposition": self.disposition,
            "winner": self.winner,
        })
    }

    fn all_passed(&self) -> bool {
        self.value()
            .as_object()
            .expect("checks is an object")
            .values()
            .all(|value| value == &Value::Bool(true))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seat {
    A,
    B,
}

impl Seat {
    const fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalReason {
    KingCaptured,
    NoLegalMove,
    RepetitionDraw,
    PerpetualCheckLoss,
    JishogiDraw,
    MaximumPlyCapped,
}

impl TerminalReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::KingCaptured => "king-captured",
            Self::NoLegalMove => "no-legal-move",
            Self::RepetitionDraw => "repetition-draw",
            Self::PerpetualCheckLoss => "perpetual-check-loss",
            Self::JishogiDraw => "jishogi-draw",
            Self::MaximumPlyCapped => "maximum-ply-capped",
        }
    }

    const fn is_draw(self) -> bool {
        matches!(
            self,
            Self::RepetitionDraw | Self::JishogiDraw | Self::MaximumPlyCapped
        )
    }
}

#[derive(Debug, Clone, Copy)]
struct Adjudication {
    reason: TerminalReason,
    winner_color: Option<Color>,
}

struct InputRecord<'a> {
    value: &'a Value,
    game: u64,
    pair: u64,
    block: u64,
    block_game: u64,
    leg: u64,
    opening_id: &'a str,
    start_sfen: &'a str,
    initial_side: Color,
    a_color: Color,
    moves: &'a [Value],
    plies: usize,
    raw_end: &'a str,
    raw_winner: Option<Seat>,
    raw_result: &'a str,
}

fn required<'a>(value: &'a Value, key: &str) -> Result<&'a Value> {
    value
        .get(key)
        .ok_or_else(|| anyhow!("missing required field {key}"))
}

fn required_str<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    required(value, key)?
        .as_str()
        .ok_or_else(|| anyhow!("{key} must be a string"))
}

fn required_u64(value: &Value, key: &str) -> Result<u64> {
    required(value, key)?
        .as_u64()
        .ok_or_else(|| anyhow!("{key} must be a nonnegative integer"))
}

fn parse_color(value: &str, field: &str) -> Result<Color> {
    match value {
        "black" => Ok(Color::Black),
        "white" => Ok(Color::White),
        _ => bail!("{field} must be black or white"),
    }
}

fn parse_seat(value: &Value, field: &str) -> Result<Option<Seat>> {
    match value {
        Value::Null => Ok(None),
        Value::String(value) if value == "A" => Ok(Some(Seat::A)),
        Value::String(value) if value == "B" => Ok(Some(Seat::B)),
        _ => bail!("{field} must be A, B, or null"),
    }
}

fn parse_record<'a>(value: &'a Value, checks: &mut Checks, max_plies: usize) -> Result<InputRecord<'a>> {
    ensure!(value.is_object(), "JSONL record must be an object");
    ensure!(
        required_str(value, "schema")? == INPUT_SCHEMA,
        "input schema differs"
    );
    checks.schema = true;

    let game = required_u64(value, "globalGameIndex")?;
    let pair = required_u64(value, "globalPairIndex")?;
    let block = required_u64(value, "block")?;
    let block_game = required_u64(value, "blockGameIndex")?;
    let leg = required_u64(value, "leg")?;
    ensure!(leg <= 1, "leg must be 0 or 1");
    ensure!(
        game == 2 * pair + leg,
        "global game/pair/leg identity differs"
    );
    ensure!(block == pair / 8, "block does not match global pair");
    ensure!(
        block_game == 2 * (pair % 8) + leg,
        "block game identity differs"
    );
    let opening_id = required_str(value, "openingId")?;
    ensure!(!opening_id.is_empty(), "openingId is empty");
    checks.identity = true;

    let start_sfen = required_str(value, "startSfen")?;
    let initial_side = parse_color(
        required_str(value, "initialSideToMove")?,
        "initialSideToMove",
    )?;
    let a_color = parse_color(required_str(value, "aColor")?, "aColor")?;
    let b_color = parse_color(required_str(value, "bColor")?, "bColor")?;
    ensure!(a_color == !b_color, "A/B colors are not opposite");
    let black_engine = required_str(value, "blackEngine")?;
    let white_engine = required_str(value, "whiteEngine")?;
    ensure!(matches!(black_engine, "A" | "B"), "blackEngine is invalid");
    ensure!(matches!(white_engine, "A" | "B"), "whiteEngine is invalid");
    ensure!(
        black_engine != white_engine,
        "engine seats are not distinct"
    );
    ensure!(
        (a_color == Color::Black && black_engine == "A" && white_engine == "B")
            || (a_color == Color::White && black_engine == "B" && white_engine == "A"),
        "engine/color seat mapping differs"
    );
    ensure!(
        (leg == 0 && a_color == Color::Black) || (leg == 1 && a_color == Color::White),
        "leg does not use the frozen A-color assignment"
    );
    checks.seats = true;

    let moves = required(value, "moves")?
        .as_array()
        .context("moves must be an array")?;
    let plies_u64 = required_u64(value, "plies")?;
    let plies = usize::try_from(plies_u64).context("plies does not fit usize")?;
    ensure!(plies == moves.len(), "plies does not equal moves length");
    ensure!(plies <= max_plies, "recorded plies exceed configured maximum");
    checks.plies = true;

    let raw_end = required_str(value, "rawEnd")?;
    ensure!(
        matches!(
            raw_end,
            "move-limit-reached"
                | "king-captured"
                | "no-legal-move"
                | "repetition-draw"
                | "perpetual-check-loss"
                | "jishogi-draw"
        ),
        "rawEnd is unknown"
    );
    let raw_winner = parse_seat(required(value, "rawWinner")?, "rawWinner")?;
    let raw_result = required_str(value, "rawResult")?;
    ensure!(
        matches!(raw_result, "a-win" | "b-win" | "draw"),
        "rawResult is unknown"
    );

    Ok(InputRecord {
        value,
        game,
        pair,
        block,
        block_game,
        leg,
        opening_id,
        start_sfen,
        initial_side,
        a_color,
        moves,
        plies,
        raw_end,
        raw_winner,
        raw_result,
    })
}

fn color_name(color: Color) -> &'static str {
    match color {
        Color::Black => "black",
        Color::White => "white",
    }
}

fn board_status_name(status: GameStatus) -> &'static str {
    match status {
        GameStatus::Won => "won",
        GameStatus::Drawn => "drawn",
        GameStatus::Ongoing => "ongoing",
    }
}

fn seat_for_color(color: Color, a_color: Color) -> Seat {
    if color == a_color {
        Seat::A
    } else {
        Seat::B
    }
}

/// Terminal precedence matches the bound game runtime: king availability,
/// history adjudication, then board status. A rules draw is only recognized
/// from `GameStatus::Drawn`; an arbitrary ongoing position is never jishogi.
fn terminal(board: &Board, history: &PositionHistory) -> Result<Option<Adjudication>> {
    ensure!(
        history.matches_current(board),
        "history/current board mismatch"
    );
    let black_king = board.has(Color::Black, Piece::King);
    let white_king = board.has(Color::White, Piece::King);
    if !black_king || !white_king {
        let winner_color = match (black_king, white_king) {
            (true, false) => Some(Color::Black),
            (false, true) => Some(Color::White),
            (false, false) => bail!("position contains neither king"),
            (true, true) => unreachable!(),
        };
        return Ok(Some(Adjudication {
            reason: TerminalReason::KingCaptured,
            winner_color,
        }));
    }

    match history.adjudication() {
        HistoryAdjudication::RepetitionDraw => {
            return Ok(Some(Adjudication {
                reason: TerminalReason::RepetitionDraw,
                winner_color: None,
            }));
        }
        HistoryAdjudication::PerpetualCheckLoss(loser) => {
            return Ok(Some(Adjudication {
                reason: TerminalReason::PerpetualCheckLoss,
                winner_color: Some(!loser),
            }));
        }
        HistoryAdjudication::Ongoing => {}
    }

    Ok(match board.status() {
        GameStatus::Won => Some(Adjudication {
            reason: TerminalReason::NoLegalMove,
            winner_color: Some(!board.side_to_move()),
        }),
        GameStatus::Drawn => Some(Adjudication {
            reason: TerminalReason::JishogiDraw,
            winner_color: None,
        }),
        GameStatus::Ongoing => None,
    })
}

fn result_for(winner: Option<Seat>) -> &'static str {
    match winner {
        Some(Seat::A) => "a-win",
        Some(Seat::B) => "b-win",
        None => "draw",
    }
}

fn history_name(value: HistoryAdjudication) -> &'static str {
    value.as_str()
}

fn base_receipt(line: usize, value: Option<&Value>, checks: &Checks) -> Value {
    let identity = value.and_then(Value::as_object);
    json!({
        "schema": OUTPUT_SCHEMA,
        "status": "failed",
        "sourceLine": line,
        "globalGameIndex": identity.and_then(|item| item.get("globalGameIndex")).cloned().unwrap_or(Value::Null),
        "globalPairIndex": identity.and_then(|item| item.get("globalPairIndex")).cloned().unwrap_or(Value::Null),
        "block": identity.and_then(|item| item.get("block")).cloned().unwrap_or(Value::Null),
        "blockGameIndex": identity.and_then(|item| item.get("blockGameIndex")).cloned().unwrap_or(Value::Null),
        "leg": identity.and_then(|item| item.get("leg")).cloned().unwrap_or(Value::Null),
        "openingId": identity.and_then(|item| item.get("openingId")).cloned().unwrap_or(Value::Null),
        "searchDispatched": false,
        "modelLoaded": false,
        "browserLaunched": false,
        "checks": checks.value(),
    })
}

fn replay_one(value: &Value, line: usize, unique: bool, max_plies: usize) -> Value {
    let mut checks = Checks {
        unique_game_identity: unique,
        ..Checks::default()
    };
    let result = (|| -> Result<Value> {
        ensure!(unique, "duplicate globalGameIndex");
        let input = parse_record(value, &mut checks, max_plies)?;

        let mut board = Board::from_sfen(input.start_sfen)
            .map_err(|error| anyhow!("invalid startSfen: {error}"))?;
        checks.sfen = true;
        ensure!(
            board.side_to_move() == input.initial_side,
            "initialSideToMove differs from startSfen"
        );
        checks.side_to_move = true;
        let mut history = PositionHistory::new(board.clone());

        for (ply, move_value) in input.moves.iter().enumerate() {
            ensure!(
                terminal(&board, &history)?.is_none(),
                "move {ply} occurs after terminal adjudication"
            );
            let text = move_value
                .as_str()
                .with_context(|| format!("move {ply} must be a string"))?;
            let mv = Move::from_str(text)
                .map_err(|error| anyhow!("invalid move {text} at ply {ply}: {error}"))?;
            checks.move_syntax = true;
            ensure!(board.is_legal(mv), "illegal move {text} at ply {ply}");
            board
                .try_play(mv)
                .map_err(|_| anyhow!("legal move {text} failed at ply {ply}"))?;
            history.push(board.clone());
        }
        checks.move_syntax = true;
        checks.move_legality = true;
        checks.no_move_after_terminal = true;
        checks.history = true;

        // Adjudicate after the final recorded move before considering the cap.
        let endpoint = terminal(&board, &history)?;
        checks.terminal_precedence = true;
        let adjudication = match endpoint {
            Some(value) => value,
            None if input.plies == max_plies => Adjudication {
                reason: TerminalReason::MaximumPlyCapped,
                winner_color: None,
            },
            None => bail!(
                "incomplete ongoing transcript: {} plies is below the exact cap",
                input.plies
            ),
        };
        let provisional_cap = input.raw_end == "move-limit-reached";
        checks.cap_semantics = !provisional_cap || input.plies == max_plies;
        ensure!(
            checks.cap_semantics,
            "move-limit-reached is valid only at exactly 200 plies"
        );

        let winner = adjudication
            .winner_color
            .map(|color| seat_for_color(color, input.a_color));
        ensure!(
            adjudication.reason.is_draw() == winner.is_none(),
            "adjudicated winner/draw semantics differ"
        );
        checks.winner = true;
        if provisional_cap {
            // The browser deliberately performs no 201st query. Its cap record
            // is provisional even when move 200 itself creates a terminal
            // position. This is the sole permitted raw-to-canonical change.
            ensure!(
                input.raw_winner.is_none() && input.raw_result == "draw",
                "provisional cap must carry null winner and draw result"
            );
        } else {
            ensure!(
                input.raw_end == adjudication.reason.as_str(),
                "rawEnd disagrees with replay: recorded {}, replayed {}",
                input.raw_end,
                adjudication.reason.as_str()
            );
            ensure!(
                input.raw_winner == winner,
                "rawWinner disagrees with replay"
            );
            ensure!(
                input.raw_result == result_for(winner),
                "rawResult disagrees with replay"
            );
        }
        checks.disposition = true;
        ensure!(
            checks.all_passed(),
            "internal replay check was not completed"
        );

        let board_status = board.status();
        let history_adjudication = history.adjudication();
        Ok(json!({
            "schema": OUTPUT_SCHEMA,
            "status": "passed",
            "sourceLine": line,
            "globalGameIndex": input.game,
            "globalPairIndex": input.pair,
            "block": input.block,
            "blockGameIndex": input.block_game,
            "leg": input.leg,
            "openingId": input.opening_id,
            "startSfen": input.start_sfen,
            "startSfenSha256": format!("{:x}", Sha256::digest(input.start_sfen.as_bytes())),
            "initialSideToMove": color_name(input.initial_side),
            "aColor": color_name(input.a_color),
            "recordedPlies": input.value.get("plies"),
            "replayedPlies": input.plies,
            "maxPlies": max_plies,
            "boardStatus": board_status_name(board_status),
            "historyAdjudication": history_name(history_adjudication),
            "terminationReason": adjudication.reason.as_str(),
            "winnerColor": adjudication.winner_color.map(color_name),
            "winner": winner.map(Seat::as_str),
            "result": result_for(winner),
            "recordedRawEnd": input.raw_end,
            "recordedRawWinner": input.raw_winner.map(Seat::as_str),
            "recordedRawResult": input.raw_result,
            "searchDispatched": false,
            "modelLoaded": false,
            "browserLaunched": false,
            "checks": checks.value(),
        }))
    })();

    match result {
        Ok(receipt) => receipt,
        Err(error) => {
            let mut receipt = base_receipt(line, Some(value), &checks);
            receipt["error"] = Value::String(error.to_string());
            receipt
        }
    }
}

fn parse_strict(line: &str) -> Result<Value> {
    let mut deserializer = serde_json::Deserializer::from_str(line);
    let value = StrictValue::deserialize(&mut deserializer)
        .map_err(|error| anyhow!("invalid strict JSON: {error}"))?;
    deserializer
        .end()
        .map_err(|error| anyhow!("trailing JSON data: {error}"))?;
    Ok(value.0)
}

fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(
        args.input != args.output,
        "input and output paths must differ"
    );
    ensure!(args.max_plies > 0 && args.max_plies <= MAX_PLIES, "max-plies must be in 1..=200");
    let input = BufReader::new(
        File::open(&args.input)
            .with_context(|| format!("cannot open input {}", args.input.display()))?,
    );
    let mut output = BufWriter::new(
        File::create(&args.output)
            .with_context(|| format!("cannot create output {}", args.output.display()))?,
    );
    let mut seen_games = HashSet::new();
    let mut records = 0usize;
    let mut failures = 0usize;

    for (index, line) in input.lines().enumerate() {
        let line_number = index + 1;
        let line = line.with_context(|| format!("read input line {line_number}"))?;
        let receipt = if line.trim().is_empty() {
            let checks = Checks::default();
            let mut receipt = base_receipt(line_number, None, &checks);
            receipt["error"] = Value::String("blank JSONL record".to_owned());
            receipt
        } else {
            match parse_strict(&line) {
                Ok(value) => {
                    let game = value.get("globalGameIndex").and_then(Value::as_u64);
                    let unique = game.is_none_or(|game| seen_games.insert(game));
                    replay_one(&value, line_number, unique, args.max_plies)
                }
                Err(error) => {
                    let checks = Checks::default();
                    let mut receipt = base_receipt(line_number, None, &checks);
                    receipt["error"] = Value::String(error.to_string());
                    receipt
                }
            }
        };
        if receipt.get("status").and_then(Value::as_str) != Some("passed") {
            failures += 1;
        }
        serde_json::to_writer(&mut output, &receipt)?;
        output.write_all(b"\n")?;
        records += 1;
    }
    output.flush()?;
    ensure!(records > 0, "input contains no JSONL records");
    if failures > 0 {
        bail!("{failures} of {records} replay records failed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Value {
        json!({
            "schema": INPUT_SCHEMA,
            "globalGameIndex": 0,
            "globalPairIndex": 0,
            "block": 0,
            "blockGameIndex": 0,
            "leg": 0,
            "openingId": "step12-development-001",
            "startSfen": "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
            "initialSideToMove": "black",
            "aColor": "black",
            "bColor": "white",
            "blackEngine": "A",
            "whiteEngine": "B",
            "moves": [],
            "plies": 0,
            "rawEnd": "move-limit-reached",
            "rawWinner": null,
            "rawResult": "draw"
        })
    }

    #[test]
    fn strict_json_rejects_duplicate_keys() {
        let error = parse_strict(r#"{"schema":"a","schema":"b"}"#).unwrap_err();
        assert!(error.to_string().contains("duplicate JSON key"));
    }

    #[test]
    fn short_ongoing_transcript_is_incomplete() {
        let receipt = replay_one(&base(), 1, true, MAX_PLIES);
        assert_eq!(receipt["status"], "failed");
        assert!(receipt["error"]
            .as_str()
            .unwrap()
            .contains("incomplete ongoing transcript"));
        assert_eq!(receipt["searchDispatched"], false);
        assert_eq!(receipt["modelLoaded"], false);
    }

    #[test]
    fn identity_and_seat_mismatches_reject() {
        let mut value = base();
        value["globalGameIndex"] = json!(1);
        let receipt = replay_one(&value, 1, true, MAX_PLIES);
        assert_eq!(receipt["status"], "failed");
        assert!(receipt["error"].as_str().unwrap().contains("identity"));

        let mut value = base();
        value["blackEngine"] = json!("B");
        let receipt = replay_one(&value, 1, true, MAX_PLIES);
        assert_eq!(receipt["status"], "failed");
        assert!(receipt["error"].as_str().unwrap().contains("seat"));
    }

    #[test]
    fn missing_disposition_rejects() {
        let mut value = base();
        value.as_object_mut().unwrap().remove("rawWinner");
        let receipt = replay_one(&value, 1, true, MAX_PLIES);
        assert_eq!(receipt["status"], "failed");
        assert!(receipt["error"].as_str().unwrap().contains("rawWinner"));
    }

    #[test]
    fn illegal_move_rejects() {
        let mut value = base();
        value["moves"] = json!(["7g7g"]);
        value["plies"] = json!(1);
        let receipt = replay_one(&value, 1, true, MAX_PLIES);
        assert_eq!(receipt["status"], "failed");
        assert!(receipt["error"].as_str().unwrap().contains("illegal move"));
        assert_eq!(receipt["checks"]["moveLegality"], false);
    }

    #[test]
    fn genuine_final_ply_terminal_precedes_cap() {
        let mut value = base();
        value["startSfen"] = json!("8k/6G2/7B1/9/9/9/9/9/K8 b R 1");
        value["moves"] = json!(["R*1b"]);
        value["plies"] = json!(1);
        value["rawEnd"] = json!("no-legal-move");
        value["rawWinner"] = json!("A");
        value["rawResult"] = json!("a-win");
        let receipt = replay_one(&value, 1, true, MAX_PLIES);
        assert_eq!(receipt["status"], "passed");
        assert_eq!(receipt["terminationReason"], "no-legal-move");
        assert_eq!(receipt["winner"], "A");
        assert_eq!(receipt["checks"]["terminalPrecedence"], true);
    }

    #[test]
    fn move_after_terminal_rejects() {
        let mut value = base();
        value["startSfen"] = json!("8k/6G2/7B1/9/9/9/9/9/K8 b R 1");
        value["moves"] = json!(["R*1b", "1a1b"]);
        value["plies"] = json!(2);
        value["rawEnd"] = json!("no-legal-move");
        value["rawWinner"] = json!("A");
        value["rawResult"] = json!("a-win");
        let receipt = replay_one(&value, 1, true, MAX_PLIES);
        assert_eq!(receipt["status"], "failed");
        assert!(receipt["error"]
            .as_str()
            .unwrap()
            .contains("after terminal adjudication"));
        assert_eq!(receipt["checks"]["noMoveAfterTerminal"], false);
    }
}
