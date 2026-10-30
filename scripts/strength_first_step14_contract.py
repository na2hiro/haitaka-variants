#!/usr/bin/env python3
"""Strict, search-free contracts for the Step 14 browser finalist worker."""

from __future__ import annotations

from collections import Counter
from fractions import Fraction
import hashlib
import json
import math
from pathlib import Path
from typing import Any, Iterable, Optional, Sequence, Tuple


EXPERIMENT_ID = "strength-first-step14-browser-finalist-20260919-v1"
PAIR_COUNT = 64
GAME_COUNT = 128
BLOCK_COUNT = 8
PAIRS_PER_BLOCK = 8
GAMES_PER_BLOCK = 16
MAX_PLIES = 200
MOVETIME_MS = 1000
ROOT_RESULT_SCHEMA = "haitaka-search-root-result-v1"
BOOTSTRAP_SEED = 2026091902
GROUP_BOOTSTRAP_SEED = 2026091903
BOOTSTRAP_REPLICATES = 10_000

TERMINAL_REASONS = {
    "king-captured", "no-legal-move", "repetition-draw",
    "perpetual-check-loss", "jishogi-draw",
}
DRAW_REASONS = {"repetition-draw", "jishogi-draw", "maximum-ply-capped"}
LOSS_REASONS = {"king-captured", "no-legal-move", "perpetual-check-loss"}
REPLAY_REASON_ALIASES = {
    "king-availability": "king-captured",
    "repetition-draw": "repetition-draw",
    "perpetual-check-loss": "perpetual-check-loss",
    "no-legal-move": "no-legal-move",
    "jishogi-draw": "jishogi-draw",
    "maximum-plies": "maximum-ply-capped",
    "maximum-ply-capped": "maximum-ply-capped",
}


class ContractError(RuntimeError):
    pass


def fail(message: str) -> None:
    raise ContractError(message)


def strict_pairs(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key {key!r}")
        result[key] = value
    return result


def load_json(path: Path) -> dict[str, Any]:
    try:
        def reject_constant(value: str) -> Any:
            raise ValueError(f"non-standard numeric constant {value}")
        value = json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=strict_pairs,
                           parse_constant=reject_constant)
    except (OSError, UnicodeError, json.JSONDecodeError, ValueError) as exc:
        fail(f"cannot read strict JSON {path}: {exc}")
    if not isinstance(value, dict):
        fail(f"JSON root is not an object: {path}")
    return value


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def stable_bytes(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":"),
                       ensure_ascii=False, allow_nan=False) + "\n").encode("utf-8")


def safe_path(root: Path, relative: str) -> Path:
    if not isinstance(relative, str) or not relative or Path(relative).is_absolute():
        fail(f"bound path must be nonempty and relative: {relative!r}")
    root = root.resolve()
    path = (root / relative).resolve()
    if path != root and root not in path.parents:
        fail(f"bound path escapes root: {relative!r}")
    return path


def require_fields(value: dict[str, Any], fields: Iterable[str], where: str) -> None:
    missing = sorted(set(fields) - value.keys())
    if missing:
        fail(f"{where}: missing required fields: {', '.join(missing)}")


def exact_int(value: Any, where: str, minimum: Optional[int] = None) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        fail(f"{where}: expected integer")
    if minimum is not None and value < minimum:
        fail(f"{where}: expected at least {minimum}")
    return value


def finite_number(value: Any, where: str, minimum: Optional[float] = None) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        fail(f"{where}: expected finite number")
    result = float(value)
    if not math.isfinite(result) or (minimum is not None and result < minimum):
        fail(f"{where}: invalid numeric value")
    return result


def exact_bool(value: Any, where: str) -> bool:
    if type(value) is not bool:
        fail(f"{where}: expected explicit boolean")
    return value


def select_openings(source: dict[str, Any]) -> list[dict[str, Any]]:
    rows = source.get("rows")
    if not isinstance(rows, list):
        fail("Step 12 schedule rows are missing")
    selected = [row for row in rows if isinstance(row, dict)
                and row.get("cell") == "early-selected" and row.get("leg") == 0
                and isinstance(row.get("pairIndex"), int)
                and not isinstance(row.get("pairIndex"), bool)
                and 0 <= row["pairIndex"] < PAIR_COUNT]
    selected.sort(key=lambda row: row["pairIndex"])
    if [row["pairIndex"] for row in selected] != list(range(PAIR_COUNT)):
        fail("Step 12 opening subset is not exactly pairIndex 0..63 once")
    result: list[dict[str, Any]] = []
    for index, row in enumerate(selected):
        expected_id = f"step12-development-{index + 1:03d}"
        require_fields(row, ("openingId", "startSfen", "sideToMove", "sourceGameId"), f"opening {index}")
        if row["openingId"] != expected_id:
            fail(f"opening {index}: expected id {expected_id!r}")
        tokens = row["startSfen"].split()
        if len(tokens) < 4 or tokens[1] not in {"b", "w"}:
            fail(f"opening {index}: malformed SFEN")
        side = "black" if tokens[1] == "b" else "white"
        if row["sideToMove"] != side:
            fail(f"opening {index}: SFEN side-to-move mismatch")
        result.append({
            "pairIndex": index, "block": index // PAIRS_PER_BLOCK,
            "blockPairIndex": index % PAIRS_PER_BLOCK,
            "openingId": row["openingId"], "startSfen": row["startSfen"],
            "initialSideToMove": side, "sourceFamily": row["sourceGameId"],
        })
    return result


def build_schedule(openings: Sequence[dict[str, Any]], model_hash: str) -> dict[str, Any]:
    if len(openings) != PAIR_COUNT:
        fail("opening count differs")
    games: list[dict[str, Any]] = []
    for opening in openings:
        pair = exact_int(opening.get("pairIndex"), "pairIndex", 0)
        for leg in (0, 1):
            games.append({
                "globalGameIndex": 2 * pair + leg,
                "globalPairIndex": pair,
                "block": pair // PAIRS_PER_BLOCK,
                "blockPairIndex": pair % PAIRS_PER_BLOCK,
                "blockGameIndex": 2 * (pair % PAIRS_PER_BLOCK) + leg,
                "leg": leg, "openingId": opening["openingId"],
                "startSfen": opening["startSfen"],
                "sourceFamily": opening["sourceFamily"],
                "initialSideToMove": opening["initialSideToMove"],
                "aColor": "black" if leg == 0 else "white",
                "bColor": "white" if leg == 0 else "black",
                "blackEngine": "A" if leg == 0 else "B",
                "whiteEngine": "B" if leg == 0 else "A",
                "modelSha256": model_hash, "movetimeMs": MOVETIME_MS,
                "maxPlies": MAX_PLIES,
            })
    if [row["globalGameIndex"] for row in games] != list(range(GAME_COUNT)):
        fail("generated game schedule is not contiguous")
    return {
        "schema": "haitaka-anhoku-strength-first-step14-schedule-v1",
        "experimentId": EXPERIMENT_ID, "pairs": PAIR_COUNT,
        "games": GAME_COUNT, "blocks": BLOCK_COUNT,
        "pairing": "same-sfen-engine-seat-swap", "rows": games,
    }


def score(result: str) -> Fraction:
    try:
        return {"a-win": Fraction(1), "draw": Fraction(1, 2),
                "b-win": Fraction(0)}[result]
    except KeyError:
        fail(f"unsupported game result {result!r}")


def expected_engine(game: dict[str, Any], move_index: int) -> tuple[str, str]:
    color = game["initialSideToMove"] if move_index % 2 == 0 else (
        "white" if game["initialSideToMove"] == "black" else "black")
    return (game["blackEngine"] if color == "black" else game["whiteEngine"], color)


TRACE_FIELDS = (
    "schema", "globalGameIndex", "globalPairIndex", "block", "blockGameIndex",
    "moveIndex", "ply", "engine", "color", "command", "requestedMs",
    "outputs", "bestMove", "gameoverReason", "gameoverReasonRaw",
    "terminalLoserColor", "terminalFlagsDerived", "rootResultSchema",
    "playMoveWasSearched", "missingMove", "emergencyFallbackUsed",
    "unsearchedMove", "elapsedMs", "provenanceEnvelopeId",
)


def validate_trace(trace: dict[str, Any], game: dict[str, Any], move_index: int,
                   envelope: str, terminal: bool) -> None:
    where = f"game {game['globalGameIndex']} trace {move_index}"
    require_fields(trace, TRACE_FIELDS, where)
    if set(trace) != set(TRACE_FIELDS):
        fail(f"{where}: trace schema has unexpected fields")
    if trace["schema"] != "haitaka-anhoku-strength-first-step14-raw-search-v1":
        fail(f"{where}: trace schema differs")
    for field in ("globalGameIndex", "globalPairIndex", "block", "blockGameIndex"):
        if trace[field] != game[field]:
            fail(f"{where}: {field} differs")
    if exact_int(trace["moveIndex"], f"{where}.moveIndex", 0) != move_index or exact_int(trace["ply"], f"{where}.ply", 1) != move_index + 1:
        fail(f"{where}: move index differs")
    engine, color = expected_engine(game, move_index)
    if trace["engine"] != engine or trace["color"] != color:
        fail(f"{where}: engine/color differs")
    if trace["command"] != "go movetime 1000" or exact_int(trace["requestedMs"], f"{where}.requestedMs", 0) != MOVETIME_MS:
        fail(f"{where}: budget differs")
    if trace["provenanceEnvelopeId"] != envelope:
        fail(f"{where}: provenance envelope differs")
    finite_number(trace["elapsedMs"], f"{where}.elapsedMs", 0)
    for field in ("playMoveWasSearched", "missingMove", "emergencyFallbackUsed", "unsearchedMove"):
        exact_bool(trace[field], f"{where}.{field}")
    exact_bool(trace["terminalFlagsDerived"], f"{where}.terminalFlagsDerived")
    outputs = trace["outputs"]
    if not isinstance(outputs, list) or not all(isinstance(line, str) for line in outputs):
        fail(f"{where}: raw USI output is missing")
    best = [line[9:].strip() for line in outputs if line.startswith("bestmove ")]
    gameover_lines = [line for line in outputs if line.startswith("info string gameover ")]
    reasons = [line[len("info string gameover "):].strip().split()[0]
               for line in gameover_lines]
    if len(best) != 1 or best[0] != trace["bestMove"]:
        fail(f"{where}: bestmove conflicts with USI")
    if len(reasons) > 1 or (reasons[0] if reasons else None) != trace["gameoverReason"]:
        fail(f"{where}: gameover reason conflicts with USI")
    if terminal:
        if len(gameover_lines) != 1 or not isinstance(trace["gameoverReasonRaw"], str):
            fail(f"{where}: terminal raw reason is missing")
        raw_reason = trace["gameoverReasonRaw"]
        if raw_reason != gameover_lines[0][len("info string gameover "):].strip().split()[0]:
            fail(f"{where}: terminal raw reason conflicts with USI")
        if trace["bestMove"] != "resign" or trace["gameoverReason"] not in TERMINAL_REASONS:
            fail(f"{where}: terminal query is not recognized")
        if trace["terminalFlagsDerived"] is not True:
            fail(f"{where}: terminal flags were not explicitly derived")
        if trace["gameoverReason"] == "perpetual-check-loss":
            if raw_reason not in {"perpetual-check-loss:black", "perpetual-check-loss:white"}:
                fail(f"{where}: perpetual-check loser color is missing")
            if trace["terminalLoserColor"] != raw_reason.rsplit(":", 1)[1]:
                fail(f"{where}: terminal loser color differs")
        elif trace["gameoverReasonRaw"] != trace["gameoverReason"] or trace["terminalLoserColor"] is not None:
            fail(f"{where}: non-perpetual terminal metadata differs")
        if trace["rootResultSchema"] is not None:
            fail(f"{where}: terminal root schema must be null")
        if trace["playMoveWasSearched"] is not False or any(trace[field] for field in
                ("missingMove", "emergencyFallbackUsed", "unsearchedMove")):
            fail(f"{where}: terminal flags differ")
    else:
        if trace["bestMove"] in {None, "", "resign"} or trace["gameoverReason"] is not None:
            fail(f"{where}: played trace is terminal or missing")
        if trace["gameoverReasonRaw"] is not None or trace["terminalLoserColor"] is not None or trace["terminalFlagsDerived"] is not False:
            fail(f"{where}: nonterminal terminal metadata differs")
        if trace["rootResultSchema"] != ROOT_RESULT_SCHEMA or trace["playMoveWasSearched"] is not True:
            fail(f"{where}: played move was not searched")
        if any(trace[field] for field in ("missingMove", "emergencyFallbackUsed", "unsearchedMove")):
            fail(f"{where}: played trace uses fallback")
        if gameover_lines:
            fail(f"{where}: nonterminal trace contains gameover output")
        info_lines = [line for line in outputs if line.startswith("info ")]
        token_list = " ".join(info_lines).split()
        def exact_token(name: str) -> str:
            values = [token_list[i + 1] for i, token in enumerate(token_list[:-1]) if token == name]
            if len(values) != 1:
                fail(f"{where}: raw USI flag {name} is absent or conflicting")
            return values[0]
        if exact_token("rootResultSchema") != trace["rootResultSchema"]:
            fail(f"{where}: root schema conflicts with raw USI")
        for name, field in (("playMoveWasSearched", "playMoveWasSearched"),
                            ("missingMove", "missingMove"),
                            ("emergencyFallbackUsed", "emergencyFallbackUsed")):
            value = exact_token(name)
            if value not in {"0", "1"} or trace[field] != (value == "1"):
                fail(f"{where}: raw USI flag {name} conflicts")


GAME_FIELDS = (
    "schema", "globalGameIndex", "globalPairIndex", "block", "blockGameIndex",
    "leg", "openingId", "startSfen", "initialSideToMove", "aColor", "bColor",
    "blackEngine", "whiteEngine", "moves", "plies", "rawEnd",
    "rawWinner", "rawResult", "searches", "provenanceEnvelopeId",
)


def validate_and_adapt_game(raw: dict[str, Any], scheduled: dict[str, Any],
                            replay: dict[str, Any], envelope: str,
                            max_plies: int = MAX_PLIES) -> dict[str, Any]:
    where = f"game {scheduled['globalGameIndex']}"
    require_fields(raw, GAME_FIELDS, where)
    if raw["schema"] != "haitaka-anhoku-strength-first-step14-raw-game-v1":
        fail(f"{where}: raw schema differs")
    for field in ("globalGameIndex", "globalPairIndex", "block", "blockGameIndex", "leg",
                  "openingId", "startSfen", "initialSideToMove", "aColor", "bColor",
                  "blackEngine", "whiteEngine"):
        if raw[field] != scheduled[field]:
            fail(f"{where}: raw {field} differs from schedule")
    if raw["provenanceEnvelopeId"] != envelope:
        fail(f"{where}: raw provenance differs")
    moves, traces = raw["moves"], raw["searches"]
    if raw["leg"] not in (0, 1) or raw["initialSideToMove"] not in ("black", "white"):
        fail(f"{where}: game seat metadata differs")
    if raw["aColor"] not in ("black", "white") or raw["bColor"] not in ("black", "white"):
        fail(f"{where}: game colors are invalid")
    if raw["aColor"] == raw["bColor"] or {raw["blackEngine"], raw["whiteEngine"]} != {"A", "B"}:
        fail(f"{where}: game seats are invalid")
    if not isinstance(moves, list) or not all(isinstance(move, str) and move for move in moves):
        fail(f"{where}: moves are invalid")
    plies = exact_int(raw["plies"], f"{where}.plies", 0)
    if plies != len(moves) or plies > max_plies or not isinstance(traces, list):
        fail(f"{where}: ply/search cardinality differs")
    if raw["rawEnd"] not in set(TERMINAL_REASONS) | {"move-limit-reached"}:
        fail(f"{where}: unsupported raw end reason")
    if raw["rawWinner"] not in (None, "A", "B"):
        fail(f"{where}: raw winner is invalid")
    expected_raw_result = "draw" if raw["rawWinner"] is None else f"{raw['rawWinner'].lower()}-win"
    if raw["rawResult"] != expected_raw_result:
        fail(f"{where}: raw result/winner disagree")
    provisional = raw["rawEnd"] == "move-limit-reached"
    if provisional:
        if plies != max_plies or len(traces) != plies:
            fail(f"{where}: provisional cap must be exactly {max_plies} moves/searches")
    else:
        if raw["rawEnd"] not in TERMINAL_REASONS or len(traces) != plies + 1:
            fail(f"{where}: terminal trace count differs")
    for index, trace in enumerate(traces):
        if not isinstance(trace, dict):
            fail(f"{where}: trace is not an object")
        validate_trace(trace, scheduled, index, envelope, terminal=(not provisional and index == plies))
        if index < plies and trace["bestMove"] != moves[index]:
            fail(f"{where}: trace move differs from played move")
    require_fields(replay, ("schema", "status", "globalGameIndex", "globalPairIndex",
                            "block", "blockGameIndex", "leg", "openingId", "startSfen",
                            "replayedPlies", "maxPlies",
                            "terminationReason", "winner", "boardStatus",
                            "historyAdjudication", "checks"), f"{where} replay")
    if replay["schema"] != "haitaka-anhoku-strength-first-step14-replay-v1":
        fail(f"{where}: replay schema differs")
    if replay["status"] != "passed" or replay["globalGameIndex"] != raw["globalGameIndex"] or replay["globalPairIndex"] != raw["globalPairIndex"] or replay["block"] != raw["block"] or replay["blockGameIndex"] != raw["blockGameIndex"] or replay["leg"] != raw["leg"] or replay["openingId"] != raw["openingId"] or replay["startSfen"] != raw["startSfen"] or exact_int(replay["replayedPlies"], f"{where} replay.replayedPlies", 0) != plies or exact_int(replay["maxPlies"], f"{where} replay.maxPlies", 1) != max_plies:
        fail(f"{where}: replay identity/status differs")
    checks = replay["checks"]
    required_checks = ("schema", "identity", "uniqueGameIdentity", "sfen", "sideToMove", "seats",
                       "plies", "moveSyntax", "moveLegality", "noMoveAfterTerminal", "history",
                       "terminalPrecedence", "capSemantics", "disposition", "winner")
    if not isinstance(checks, dict) or set(checks) != set(required_checks) or any(checks[name] is not True for name in required_checks):
        fail(f"{where}: replay checks did not all pass")
    replay_reason = REPLAY_REASON_ALIASES.get(replay["terminationReason"])
    if replay_reason is None:
        fail(f"{where}: unsupported replay terminal reason")
    if provisional:
        reason = replay_reason
        if reason == "maximum-ply-capped":
            if replay.get("boardStatus") != "ongoing" or replay.get("historyAdjudication") != "ongoing":
                fail(f"{where}: cap replay is not ongoing")
        elif replay.get("boardStatus") == "ongoing" and replay.get("historyAdjudication") == "ongoing":
            fail(f"{where}: short ongoing transcript was accepted as terminal")
    else:
        reason = raw["rawEnd"]
        if replay_reason != reason or replay["winner"] != raw["rawWinner"]:
            fail(f"{where}: terminal raw/replay disagreement")
    winner = replay["winner"]
    if reason in DRAW_REASONS:
        if winner is not None:
            fail(f"{where}: draw has a winner")
        result = "draw"
    elif reason in LOSS_REASONS:
        if winner not in {"A", "B"}:
            fail(f"{where}: decisive result has no winner")
        result = f"{winner.lower()}-win"
    else:
        fail(f"{where}: unhandled canonical reason {reason!r}")
    if provisional and raw["rawWinner"] is not None:
        fail(f"{where}: provisional cap must not publish a winner")
    return {
        **{field: raw[field] for field in ("globalGameIndex", "globalPairIndex", "block",
                                           "blockGameIndex", "leg", "openingId", "startSfen",
                                           "initialSideToMove", "aColor", "bColor",
                                           "blackEngine", "whiteEngine")},
        "schema": "haitaka-anhoku-strength-first-step14-canonical-game-v1",
        "moves": moves, "plies": plies, "winner": winner, "result": result,
        "terminationReason": reason, "capped": reason == "maximum-ply-capped",
        "rawEnd": raw["rawEnd"], "searches": traces,
    }


def validate_block(raw_report: dict[str, Any], replay_report: dict[str, Any],
                   schedule: dict[str, Any], block: int, envelope: str,
                   fixture_max_plies: Optional[int] = None) -> dict[str, Any]:
    require_fields(raw_report, ("schema", "status", "provenance", "rawGames"), "raw report")
    if raw_report["schema"] != "haitaka-anhoku-strength-first-step14-browser-report-v1" or raw_report["status"] != "passed":
        fail("raw report identity/status differs")
    if raw_report["provenance"].get("envelopeId") != envelope:
        fail("raw report acknowledgement differs")
    rows = [row for row in schedule.get("rows", []) if row.get("block") == block]
    games = raw_report["rawGames"]
    replays = replay_report.get("records")
    if not isinstance(games, list) or not isinstance(replays, list) or len(games) != len(rows) or len(replays) != len(rows):
        fail("block raw/replay/schedule cardinality differs")
    by_game: dict[int, dict[str, Any]] = {}
    for replay in replays:
        index = replay.get("globalGameIndex") if isinstance(replay, dict) else None
        if isinstance(index, bool) or not isinstance(index, int) or index in by_game:
            fail("duplicate or invalid replay game identity")
        by_game[index] = replay
    canonical = []
    max_plies = fixture_max_plies if fixture_max_plies is not None else MAX_PLIES
    for row, raw in zip(rows, games):
        if not isinstance(raw, dict) or row["globalGameIndex"] not in by_game:
            fail("missing or misordered raw/replay game")
        canonical.append(validate_and_adapt_game(raw, row, by_game[row["globalGameIndex"]], envelope, max_plies))
    if fixture_max_plies is None and max_plies != MAX_PLIES:
        fail("production cap differs")
    return {"schema": "haitaka-anhoku-strength-first-step14-block-validation-v1",
            "status": "passed", "block": block, "games": canonical}


class SplitMix64:
    def __init__(self, seed: int):
        self.state = seed & ((1 << 64) - 1)

    def next(self) -> int:
        self.state = (self.state + 0x9E3779B97F4A7C15) & ((1 << 64) - 1)
        value = self.state
        value = ((value ^ (value >> 30)) * 0xBF58476D1CE4E5B9) & ((1 << 64) - 1)
        value = ((value ^ (value >> 27)) * 0x94D049BB133111EB) & ((1 << 64) - 1)
        return value ^ (value >> 31)

    def index(self, bound: int) -> int:
        limit = (1 << 64) - ((1 << 64) % bound)
        while True:
            value = self.next()
            if value < limit:
                return value % bound


def type7(values: Sequence[float], probability: float) -> float:
    ordered = sorted(values)
    position = probability * (len(ordered) - 1)
    low, high = math.floor(position), math.ceil(position)
    return ordered[low] if low == high else ordered[low] * (high - position) + ordered[high] * (position - low)


def analyze(games: Sequence[dict[str, Any]]) -> dict[str, Any]:
    if len(games) != GAME_COUNT or [game["globalGameIndex"] for game in games] != list(range(GAME_COUNT)):
        fail("analysis requires the complete ordered 128-game schedule")
    pair_scores: list[Fraction] = []
    histogram: Counter[str] = Counter()
    for pair in range(PAIR_COUNT):
        legs = games[2 * pair:2 * pair + 2]
        if any(game["globalPairIndex"] != pair for game in legs):
            fail(f"pair {pair}: game identity differs")
        value = (score(legs[0]["result"]) + score(legs[1]["result"])) / 2
        pair_scores.append(value)
        histogram[str(value)] += 1
    wins = sum(game["result"] == "a-win" for game in games)
    draws = sum(game["result"] == "draw" for game in games)
    losses = GAME_COUNT - wins - draws
    capped = [game for game in games if game["capped"] is True]
    genuine_points = sum((score(game["result"]) for game in games if not game["capped"]), Fraction())
    lower, upper = genuine_points / GAME_COUNT, (genuine_points + len(capped)) / GAME_COUNT
    point = Fraction(2 * wins + draws, 2 * GAME_COUNT)
    rng = SplitMix64(BOOTSTRAP_SEED)
    vector: list[int] = []
    samples: list[float] = []
    for replicate in range(BOOTSTRAP_REPLICATES):
        indexes = [rng.index(PAIR_COUNT) for _ in range(PAIR_COUNT)]
        if replicate == 0:
            vector = indexes[:16]
        samples.append(float(sum((pair_scores[index] for index in indexes), Fraction()) / PAIR_COUNT))
    favorable = 2 * wins + draws > GAME_COUNT and lower > Fraction(1, 2)
    return {
        "schema": "haitaka-anhoku-strength-first-step14-analysis-v1", "status": "passed",
        "experimentId": EXPERIMENT_ID,
        "score": {"wins": wins, "draws": draws, "losses": losses,
                  "exact": str(point), "decimal": float(point), "twoWPlusD": 2 * wins + draws},
        "pairScoreHistogram": dict(sorted(histogram.items())),
        "bootstrap": {"seed": BOOTSTRAP_SEED, "replicates": BOOTSTRAP_REPLICATES,
                      "rng": "splitmix64-rejection-sampling", "vectorPrefix": vector,
                      "quantile": "linear-type-7", "lower95": type7(samples, .025),
                      "upper95": type7(samples, .975)},
        "cappedSensitivity": {"cappedGames": len(capped),
                              "cappedPairs": len({game["globalPairIndex"] for game in capped}),
                              "lowerExact": str(lower), "upperExact": str(upper),
                              "lower": float(lower), "upper": float(upper)},
        "routing": "favorable" if favorable else "investigate-browser-search-gap",
        "flags": {"confirmationAuthorized": False, "deploymentAuthorized": False,
                  "gpuRentalAuthorized": False, "handcraftedRemainsProduction": True},
    }
