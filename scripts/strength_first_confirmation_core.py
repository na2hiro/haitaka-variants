#!/usr/bin/env python3
"""Reusable Step 16 confirmation statistics, validation, and recovery core.

The statistical guarantee implemented here is deliberately narrow.  For each
independent source group the two candidate-oriented game scores are averaged,
and the fixed (not sequential) final-sample test uses

    product(1 + (X_i - m) / 8) > 20.

Independence (or an explicitly justified conditional-mean design) is an input
assumption; identifiers and random ordering do not establish it.  Sampling
without replacement needs its own justification before this test is used.
"""

from __future__ import annotations

from collections import Counter
from dataclasses import dataclass
import fcntl
import hashlib
import json
import math
import os
from pathlib import Path
import re
import tempfile
from typing import Any, Callable, Iterable, Mapping, MutableMapping, Sequence


METHOD = "fixed-bet-one-sided-v1"
LAMBDA_NUMERATOR = 1
LAMBDA_DENOMINATOR = 8
E_THRESHOLD = 20
SCORE_QUARTERS = frozenset(range(5))
TERMINAL_REASONS = frozenset(
    {"king-captured", "no-legal-move", "repetition-draw",
     "perpetual-check-loss", "jishogi-draw", "maximum-ply-capped"}
)
DRAW_REASONS = frozenset(
    {"repetition-draw", "jishogi-draw", "maximum-ply-capped"}
)


class ConfirmationError(ValueError):
    """An input cannot be admitted to the confirmation evidence chain."""


def _fail(message: str) -> None:
    raise ConfirmationError(message)


def canonical_json(value: Any) -> bytes:
    try:
        return (json.dumps(value, sort_keys=True, separators=(",", ":"),
                           ensure_ascii=False, allow_nan=False) + "\n").encode()
    except (TypeError, ValueError) as exc:
        _fail(f"value is not canonical JSON: {exc}")


def digest_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def digest_value(value: Any) -> str:
    return digest_bytes(canonical_json(value))


def _strict_int(value: Any, where: str, minimum: int | None = None) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        _fail(f"{where} must be an integer (booleans are invalid)")
    if minimum is not None and value < minimum:
        _fail(f"{where} must be at least {minimum}")
    return value


def _nonempty_string(value: Any, where: str) -> str:
    if not isinstance(value, str) or not value or "\x00" in value:
        _fail(f"{where} must be a nonempty string")
    return value


def _sha256_string(value: Any, where: str) -> str:
    text = _nonempty_string(value, where)
    if re.fullmatch(r"[0-9a-f]{64}", text) is None:
        _fail(f"{where} must be a lowercase SHA-256 digest")
    return text


def score_quarters(score: Any, where: str = "score") -> int:
    """Convert exactly one allowed score to quarters, rejecting bool/nonfinite."""
    if isinstance(score, bool) or not isinstance(score, (int, float)):
        _fail(f"{where} must be one of 0, 0.25, 0.5, 0.75, 1")
    value = float(score)
    if not math.isfinite(value):
        _fail(f"{where} must be finite")
    quarters = round(value * 4)
    if quarters not in SCORE_QUARTERS or value * 4 != quarters:
        _fail(f"{where} must be one of 0, 0.25, 0.5, 0.75, 1")
    return quarters


def _quarter_counts(scores: Sequence[Any]) -> Counter[int]:
    if isinstance(scores, (str, bytes)):
        _fail("scores must be a sequence of numeric values")
    return Counter(score_quarters(value, f"scores[{index}]")
                   for index, value in enumerate(scores))


def strict_integer_threshold(product: Any, threshold: Any) -> bool:
    """Return the strict decision used by exact tests; equality always fails."""
    product = _strict_int(product, "product", 0)
    threshold = _strict_int(threshold, "threshold", 0)
    return product > threshold


def log_e_value(scores: Sequence[Any], m: float) -> float:
    """Return log(E_n(m)); intended for reporting, never the m=.5 decision."""
    if isinstance(m, bool) or not isinstance(m, (int, float)) or not math.isfinite(float(m)):
        _fail("m must be finite and in [0,1]")
    m = float(m)
    if not 0 <= m <= 1:
        _fail("m must be in [0,1]")
    counts = _quarter_counts(scores)
    return math.fsum(count * math.log1p((quarter / 4 - m) / 8)
                     for quarter, count in counts.items())


def exact_half_comparison(scores: Sequence[Any]) -> int:
    """Compare E_n(0.5) to 20 exactly; return -1, 0, or 1."""
    counts = _quarter_counts(scores)
    n = sum(counts.values())
    product = math.prod(pow(30 + quarter, count) for quarter, count in counts.items())
    threshold = E_THRESHOLD * pow(32, n)
    return strict_integer_threshold(product, threshold) - (product < threshold)


def rejects_half(scores: Sequence[Any]) -> bool:
    """Exact strict primary decision.  Empty and one-row samples are valid."""
    return exact_half_comparison(scores) > 0


def _dyadic_comparison(counts: Mapping[int, int], numerator: int, bits: int) -> int:
    """Compare E_n(numerator/2**bits) to 20 using integers only."""
    scale = 1 << bits
    n = sum(counts.values())
    product = math.prod(pow((32 + quarter) * scale - 4 * numerator, count)
                        for quarter, count in counts.items())
    threshold = E_THRESHOLD * pow(32 * scale, n)
    return (product > threshold) - (product < threshold)


def conservative_lower_bound(scores: Sequence[Any], bits: int = 40) -> dict[str, Any]:
    """Invert on a dyadic grid and round downward.

    The returned endpoint is the greatest grid point whose null is strictly
    rejected.  Thus it never exceeds the continuous endpoint, and the stated
    error is less than or equal to one grid interval.  Endpoint decisions are
    verified with integer products, not floating point logs.
    """
    bits = _strict_int(bits, "bits", 1)
    if bits > 60:
        _fail("bits must not exceed 60")
    counts = _quarter_counts(scores)
    scale = 1 << bits
    if _dyadic_comparison(counts, 0, bits) <= 0:
        numerator = 0
    elif _dyadic_comparison(counts, scale, bits) > 0:
        numerator = scale
    else:
        low, high = 0, scale
        while high - low > 1:
            middle = (low + high) // 2
            if _dyadic_comparison(counts, middle, bits) > 0:
                low = middle
            else:
                high = middle
        numerator = low
        if _dyadic_comparison(counts, numerator, bits) <= 0:
            _fail("internal inversion error: reported endpoint is not rejected")
        if numerator < scale and _dyadic_comparison(counts, numerator + 1, bits) > 0:
            _fail("internal inversion error: endpoint is not maximal")
    exact_half = _dyadic_comparison(counts, scale // 2, bits) > 0
    decision_consistent = exact_half == (numerator > scale // 2)
    if not decision_consistent:
        _fail("dyadic precision is insufficient to report a lower bound consistent with the exact half decision")
    return {
        "method": METHOD,
        "n": sum(counts.values()),
        "quartersHistogram": {str(q): counts.get(q, 0) for q in range(5)},
        "dyadicNumerator": numerator,
        "dyadicBits": bits,
        "lowerBound": numerator / scale,
        "rounding": "downward-conservative",
        "absoluteNumericalErrorAtMost": 1 / scale,
        "rejectMeanAtMostHalf": exact_half,
        "decisionConsistent": decision_consistent,
    }


def analyze_game_legs(legs: Sequence[Mapping[str, Any]], bits: int = 40) -> dict[str, Any]:
    """Analyze paired legs, pessimistically zeroing only capped candidate legs."""
    if not isinstance(legs, Sequence) or isinstance(legs, (str, bytes)):
        _fail("legs must be a sequence")
    pairs: dict[str, dict[int, tuple[int, bool]]] = {}
    for index, leg in enumerate(legs):
        if not isinstance(leg, Mapping):
            _fail(f"legs[{index}] must be an object")
        pair_id = _nonempty_string(leg.get("pairId"), f"legs[{index}].pairId")
        number = _strict_int(leg.get("leg"), f"legs[{index}].leg")
        if number not in (0, 1):
            _fail(f"legs[{index}].leg must be 0 or 1")
        if number in pairs.setdefault(pair_id, {}):
            _fail(f"duplicate leg {pair_id}/{number}")
        capped = leg.get("capped")
        if not isinstance(capped, bool):
            _fail(f"legs[{index}].capped must be boolean")
        pairs[pair_id][number] = (score_quarters(leg.get("candidateScore"),
                                                 f"legs[{index}].candidateScore"), capped)
    ordinary: list[float] = []
    pessimistic: list[float] = []
    for pair_id in sorted(pairs):
        pair = pairs[pair_id]
        if set(pair) != {0, 1}:
            _fail(f"pair {pair_id!r} does not contain exactly legs 0 and 1")
        ordinary.append((pair[0][0] + pair[1][0]) / 8)
        pessimistic.append(((0 if pair[0][1] else pair[0][0]) +
                            (0 if pair[1][1] else pair[1][0])) / 8)
    ordinary_result = conservative_lower_bound(ordinary, bits)
    pessimistic_result = conservative_lower_bound(pessimistic, bits)
    return {
        "method": METHOD,
        "pairs": len(pairs),
        "ordinary": ordinary_result,
        "pessimistic": pessimistic_result,
        "pass": ordinary_result["rejectMeanAtMostHalf"] and
                pessimistic_result["rejectMeanAtMostHalf"],
    }


def _sfen_side(sfen: str) -> str:
    fields = _nonempty_string(sfen, "sfen").split()
    if len(fields) != 4 or fields[1] not in {"b", "w"}:
        _fail("SFEN must have four fields and a b/w side-to-move field")
    return "black" if fields[1] == "b" else "white"


def validate_reference_graph(nodes: Mapping[str, Mapping[str, Any]], roots: Iterable[str]) -> set[str]:
    """Validate a transitive, content-addressed JSON evidence graph."""
    if not isinstance(nodes, Mapping):
        _fail("referenceGraph must be an object")
    visiting: set[str] = set()
    visited: set[str] = set()

    def visit(node_id: str) -> None:
        node_id = _nonempty_string(node_id, "reference id")
        if node_id in visiting:
            _fail(f"reference graph cycle at {node_id!r}")
        if node_id in visited:
            return
        node = nodes.get(node_id)
        if not isinstance(node, Mapping):
            _fail(f"missing referenced node {node_id!r}")
        payload = node.get("payload")
        if digest_value(payload) != _sha256_string(node.get("sha256"), f"node {node_id}.sha256"):
            _fail(f"hash mismatch for referenced node {node_id!r}")
        refs = node.get("refs", [])
        if not isinstance(refs, list) or any(not isinstance(ref, str) for ref in refs):
            _fail(f"node {node_id!r}.refs must be a string array")
        visiting.add(node_id)
        for ref in refs:
            visit(ref)
        visiting.remove(node_id)
        visited.add(node_id)

    for root in roots:
        visit(root)
    return visited


def _candidate_score(winner: Any) -> float:
    if winner == "candidate":
        return 1.0
    if winner == "handcrafted":
        return 0.0
    if winner is None:
        return 0.5
    _fail(f"invalid canonical winner {winner!r}")


def analyze_confirmation(contract: Mapping[str, Any], schedule: Sequence[Mapping[str, Any]],
                         records: Sequence[Mapping[str, Any]], models: Mapping[str, Any],
                         reference_graph: Mapping[str, Mapping[str, Any]], *,
                         real_gate: bool = True, bits: int = 40) -> dict[str, Any]:
    """Validate the full fixed schedule/evidence boundary, then analyze A and B."""
    if not isinstance(contract, Mapping) or contract.get("method") != METHOD:
        _fail(f"contract.method must be {METHOD!r}")
    if not isinstance(models, Mapping) or set(models) != {"A", "B"}:
        _fail("models must contain exactly A and B")
    expected_counts = {name: _strict_int(contract.get(f"n{name}"), f"contract.n{name}", 0)
                       for name in ("A", "B")}
    requested_ms = _strict_int(contract.get("requestedMs"), "contract.requestedMs", 1)
    if requested_ms != 1000:
        _fail("confirmation contract must request exactly 1000 ms")
    schedule_hash = _sha256_string(contract.get("scheduleSha256"), "contract.scheduleSha256")
    if digest_value(schedule) != schedule_hash:
        _fail("schedule hash mismatch")
    runtime_hash = _sha256_string(contract.get("runtimeSha256"), "contract.runtimeSha256")
    adapter_hash = _sha256_string(contract.get("adapterSha256"), "contract.adapterSha256")
    model_hashes: dict[str, str] = {}
    for model in ("A", "B"):
        identity = models[model]
        if not isinstance(identity, Mapping) or identity.get("id") != model:
            _fail(f"invalid model identity for {model}")
        model_hashes[model] = _sha256_string(identity.get("sha256"), f"models.{model}.sha256")

    expected: dict[tuple[str, str, int], Mapping[str, Any]] = {}
    pair_rows: dict[tuple[str, str], list[Mapping[str, Any]]] = {}
    source_group_owner: dict[str, tuple[str, str]] = {}
    for index, row in enumerate(schedule):
        if not isinstance(row, Mapping):
            _fail(f"schedule[{index}] must be an object")
        model = row.get("model")
        if model not in {"A", "B"}:
            _fail(f"schedule[{index}].model is invalid")
        pair_id = _nonempty_string(row.get("pairId"), f"schedule[{index}].pairId")
        leg = _strict_int(row.get("leg"), f"schedule[{index}].leg")
        if leg not in (0, 1):
            _fail(f"schedule[{index}].leg must be 0 or 1")
        key = (model, pair_id, leg)
        if key in expected:
            _fail(f"duplicate scheduled game {key!r}")
        source_group = _nonempty_string(row.get("sourceGroupId"), f"schedule[{index}].sourceGroupId")
        owner = (model, pair_id)
        if source_group in source_group_owner and source_group_owner[source_group] != owner:
            _fail(f"source group {source_group!r} is assigned to multiple pairs")
        source_group_owner[source_group] = owner
        sfen = _nonempty_string(row.get("sfen"), f"schedule[{index}].sfen")
        if row.get("initialSideToMove") != _sfen_side(sfen):
            _fail(f"schedule[{index}] initial side conflicts with SFEN")
        wanted_color = "black" if leg == 0 else "white"
        if row.get("candidateColor") != wanted_color:
            _fail(f"schedule[{index}] does not implement the required seat swap")
        if _strict_int(row.get("requestedMs"), f"schedule[{index}].requestedMs") != requested_ms:
            _fail(f"schedule[{index}] requested time differs")
        if row.get("modelSha256") != model_hashes[model] or row.get("runtimeSha256") != runtime_hash or row.get("adapterSha256") != adapter_hash:
            _fail(f"schedule[{index}] execution identity differs")
        fixture_only = row.get("fixtureOnly")
        if not isinstance(fixture_only, bool):
            _fail(f"schedule[{index}].fixtureOnly must be boolean")
        if real_gate and fixture_only:
            _fail("fixture-only schedule is forbidden at a real gate")
        expected[key] = row
        pair_rows.setdefault((model, pair_id), []).append(row)
    for pair, rows in pair_rows.items():
        if len(rows) != 2 or {row["leg"] for row in rows} != {0, 1}:
            _fail(f"scheduled pair {pair!r} is incomplete")
        if len({row["sfen"] for row in rows}) != 1 or len({row["sourceGroupId"] for row in rows}) != 1:
            _fail(f"scheduled pair {pair!r} changes SFEN or source group")
    actual_counts = {model: sum(1 for key in pair_rows if key[0] == model) for model in ("A", "B")}
    if actual_counts != expected_counts or len(schedule) != 2 * sum(expected_counts.values()):
        _fail(f"schedule pair counts differ: expected {expected_counts}, got {actual_counts}")

    seen: set[tuple[str, str, int]] = set()
    analysis_legs: dict[str, list[dict[str, Any]]] = {"A": [], "B": []}
    graph_roots: list[str] = []
    for index, record in enumerate(records):
        if not isinstance(record, Mapping):
            _fail(f"records[{index}] must be an object")
        key = (record.get("model"), record.get("pairId"), record.get("leg"))
        if key not in expected or key in seen:
            _fail(f"record {index} is duplicate, extra, or has an invalid identity")
        seen.add(key)
        row = expected[key]
        for field in ("sourceGroupId", "sfen", "initialSideToMove", "candidateColor",
                      "requestedMs", "modelSha256", "runtimeSha256", "adapterSha256",
                      "fixtureOnly"):
            if record.get(field) != row.get(field):
                _fail(f"record {index}.{field} differs from frozen schedule")
        if record.get("disposition") != "accepted":
            _fail(f"record {index} is not accepted; excluded games cannot be analyzed")
        validity = record.get("validity")
        required_validity = {"completeTrace", "legal", "searched", "noFallback",
                             "provenance", "timing"}
        if not isinstance(validity, Mapping) or set(validity) != required_validity or not all(value is True for value in validity.values()):
            _fail(f"record {index} has an incomplete validity gate")
        replay = record.get("replay")
        raw = record.get("raw")
        if not isinstance(replay, Mapping) or replay.get("status") != "passed" or replay.get("searchFree") is not True:
            _fail(f"record {index} lacks completed search-free replay")
        if not isinstance(raw, Mapping):
            _fail(f"record {index}.raw must be an object")
        winner = raw.get("winner")
        reason = raw.get("terminationReason")
        if reason not in TERMINAL_REASONS:
            _fail(f"record {index} has invalid terminal semantics")
        if reason in DRAW_REASONS and winner is not None:
            _fail(f"record {index} draw reason conflicts with winner")
        if reason not in DRAW_REASONS and winner not in {"candidate", "handcrafted"}:
            _fail(f"record {index} decisive reason lacks winner")
        if replay.get("winner") != winner or replay.get("terminationReason") != reason:
            _fail(f"record {index} raw/replay winner or termination differs")
        capped = raw.get("capped")
        if not isinstance(capped, bool) or capped != (reason == "maximum-ply-capped"):
            _fail(f"record {index} capped status conflicts with termination")
        raw_hash = _sha256_string(record.get("rawSha256"), f"records[{index}].rawSha256")
        replay_hash = _sha256_string(record.get("replaySha256"), f"records[{index}].replaySha256")
        if raw_hash != digest_value(raw) or replay_hash != digest_value(replay):
            _fail(f"record {index} raw/replay content hash differs")
        reference = _nonempty_string(record.get("referenceRoot"), f"records[{index}].referenceRoot")
        root_node = reference_graph.get(reference)
        payload = root_node.get("payload") if isinstance(root_node, Mapping) else None
        if not isinstance(payload, Mapping) or payload.get("rawSha256") != raw_hash or payload.get("replaySha256") != replay_hash:
            _fail(f"record {index} reference root does not bind raw/replay hashes")
        graph_roots.append(reference)
        analysis_legs[key[0]].append({"pairId": key[1], "leg": key[2],
                                      "candidateScore": _candidate_score(winner), "capped": capped})
    if seen != set(expected):
        _fail(f"record set is incomplete; missing={sorted(set(expected) - seen)!r}")
    reached = validate_reference_graph(reference_graph, graph_roots)
    results = {model: analyze_game_legs(analysis_legs[model], bits) for model in ("A", "B")}
    return {
        "method": METHOD,
        "fixedSampleFinalOnly": True,
        "scheduleSha256": schedule_hash,
        "validatedReferenceNodes": sorted(reached),
        "models": results,
        "conjunctionPass": results["A"]["pass"] and results["B"]["pass"],
    }


def safe_reference(root: Path, relative: str) -> Path:
    relative = _nonempty_string(relative, "relative path")
    if Path(relative).is_absolute():
        _fail("absolute artifact references are forbidden")
    root = root.resolve()
    candidate = (root / relative).resolve()
    if candidate != root and root not in candidate.parents:
        _fail("artifact reference escapes its root")
    return candidate


def validate_source_policy(rows: Sequence[Mapping[str, Any]], *, real_gate: bool = True) -> dict[str, Any]:
    """Validate frozen source groups and normalized-family exclusions."""
    seen_ids: set[str] = set()
    seen_families: set[str] = set()
    seen_model_groups: dict[str, set[str]] = {"A": set(), "B": set()}
    for index, row in enumerate(rows):
        if not isinstance(row, Mapping):
            _fail(f"source row {index} must be an object")
        row_id = _nonempty_string(row.get("id"), f"source[{index}].id")
        family = _nonempty_string(row.get("canonicalFamily"), f"source[{index}].canonicalFamily")
        group = _nonempty_string(row.get("sourceGroupId"), f"source[{index}].sourceGroupId")
        model = row.get("model")
        if model not in {"A", "B"}:
            _fail(f"source[{index}].model is invalid")
        if row_id in seen_ids or family in seen_families:
            _fail("source collision by id or color/reflection-normalized family")
        if group in seen_model_groups["A"] or group in seen_model_groups["B"]:
            _fail("source groups (the independent sampling units) overlap A/B or rows")
        histories = row.get("history")
        if not isinstance(histories, list) or any(not isinstance(move, str) or not move for move in histories):
            _fail(f"source[{index}].history must be a move-string array")
        if not isinstance(row.get("fixtureOnly"), bool):
            _fail(f"source[{index}].fixtureOnly must be boolean")
        if real_gate and row["fixtureOnly"]:
            _fail("fixture-only source is forbidden at a real gate")
        seen_ids.add(row_id)
        seen_families.add(family)
        seen_model_groups[model].add(group)
    return {"rows": len(rows), "highestIndependentSamplingUnit": "sourceGroupId",
            "historyDistinct": True, "modelSourceGroups": {k: sorted(v) for k, v in seen_model_groups.items()}}


@dataclass(frozen=True)
class ChildObservation:
    ownership: str  # owned, unknown, foreign
    alive: bool | None
    confirmed_dead_at: float | None = None


def reconcile_worker(started_at: float, last_heartbeat_at: float | None,
                     observation: ChildObservation, now: float,
                     previous_charge: float = 0.0) -> dict[str, Any]:
    """Conservatively account an interrupted worker and decide dispatch safety."""
    values = (started_at, now, previous_charge)
    if any(isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) for value in values):
        _fail("worker times must be finite numbers")
    if last_heartbeat_at is not None and (isinstance(last_heartbeat_at, bool) or
       not isinstance(last_heartbeat_at, (int, float)) or not math.isfinite(last_heartbeat_at)):
        _fail("heartbeat time must be finite")
    if observation.ownership not in {"owned", "unknown", "foreign"}:
        _fail("invalid child ownership state")
    endpoint = max(started_at, now, last_heartbeat_at or started_at)
    if observation.confirmed_dead_at is not None:
        endpoint = max(endpoint, observation.confirmed_dead_at)
    conservative_charge = max(previous_charge, endpoint - started_at)
    verified_active = max(0.0, (last_heartbeat_at or started_at) - started_at)
    can_launch = observation.ownership == "owned" and observation.alive is False and observation.confirmed_dead_at is not None
    return {"conservativeChargeSeconds": conservative_charge,
            "verifiedActiveSeconds": verified_active,
            "canLaunchReplacement": can_launch,
            "blockedReason": None if can_launch else "child-ownership-or-liveness-unresolved"}


class AppendOnlyJournal:
    """Fsync'd JSONL accounting that survives revisions and failed commands."""

    def __init__(self, path: Path):
        self.path = Path(path)

    def append(self, event: Mapping[str, Any]) -> None:
        data = canonical_json(dict(event))
        self.path.parent.mkdir(parents=True, exist_ok=True)
        fd = os.open(self.path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
        try:
            fcntl.flock(fd, fcntl.LOCK_EX)
            os.write(fd, data)
            os.fsync(fd)
        finally:
            os.close(fd)

    def events(self) -> list[dict[str, Any]]:
        if not self.path.exists():
            return []
        result: list[dict[str, Any]] = []
        for number, line in enumerate(self.path.read_text(encoding="utf-8").splitlines(), 1):
            try:
                value = json.loads(line)
            except json.JSONDecodeError as exc:
                _fail(f"journal line {number} is invalid: {exc}")
            if not isinstance(value, dict):
                _fail(f"journal line {number} is not an object")
            result.append(value)
        return result

    def totals(self) -> dict[str, int]:
        totals = {"attemptedCommands": 0, "completedCommands": 0,
                  "physicalGames": 0, "acceptedGames": 0}
        for event in self.events():
            kind = event.get("event")
            if kind == "attempt":
                totals["attemptedCommands"] += 1
            elif kind == "complete":
                totals["completedCommands"] += 1
            for field in ("physicalGames", "acceptedGames"):
                value = event.get(field, 0)
                totals[field] += _strict_int(value, f"journal.{field}", 0)
        return totals


class ExclusiveInvocation:
    def __init__(self, path: Path):
        self.path = Path(path)
        self.fd: int | None = None

    def __enter__(self) -> "ExclusiveInvocation":
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.fd = os.open(self.path, os.O_RDWR | os.O_CREAT, 0o600)
        try:
            fcntl.flock(self.fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            os.close(self.fd)
            self.fd = None
            _fail("duplicate invocation: exclusive lock is held")
        return self

    def __exit__(self, *_: Any) -> None:
        assert self.fd is not None
        fcntl.flock(self.fd, fcntl.LOCK_UN)
        os.close(self.fd)
        self.fd = None


def reconcile_accepted(receipt: Mapping[str, Any] | None, status: Mapping[str, Any] | None) -> dict[str, Any]:
    """Make an immutable accepted receipt authoritative over stale coordinator state."""
    if receipt is None:
        return dict(status or {})
    if receipt.get("accepted") is not True:
        _fail("acceptance receipt is not accepted")
    immutable = {key: receipt.get(key) for key in ("scheduleSha256", "rawSha256", "replaySha256")}
    if any(not isinstance(value, str) for value in immutable.values()):
        _fail("acceptance receipt lacks immutable graph hashes")
    if status and status.get("accepted") is True:
        for key, value in immutable.items():
            if status.get(key) not in {None, value}:
                _fail("stale coordinator attempted to overwrite accepted evidence")
    return {"accepted": True, **immutable, "dispatch": False, "replay": False}


class GameMessageHandler:
    """Stateful parser for provenance acknowledgement and archived USI output."""

    def __init__(self, expected_ack: str, candidate_color: str,
                 side_to_move: str | None = None):
        self.expected_ack = _nonempty_string(expected_ack, "expected acknowledgement")
        if candidate_color not in {"black", "white"}:
            _fail("candidate color must be black or white")
        if side_to_move not in {None, "black", "white"}:
            _fail("side to move must be black, white, or omitted")
        self.candidate_color = candidate_color
        self.side_to_move = side_to_move
        self.provenance_acked = False
        self.result: dict[str, Any] | None = None

    def handle(self, message: str) -> dict[str, Any] | None:
        message = _nonempty_string(message, "message").strip()
        if message.startswith("provenance-ack "):
            token = message.split(" ", 1)[1]
            if token != self.expected_ack:
                _fail("wrong provenance acknowledgement")
            self.provenance_acked = True
            return None
        if message in {"readyok", "usiok"} or message.startswith("ack "):
            return None
        prefix = "info string gameover "
        if not message.startswith(prefix):
            return None
        if not self.provenance_acked:
            _fail("result arrived before provenance acknowledgement")
        if self.result is not None:
            _fail("duplicate gameover result")
        raw = message[len(prefix):].strip()
        reason, separator, loser = raw.partition(":")
        if reason not in TERMINAL_REASONS:
            _fail("unknown gameover reason")
        if reason == "perpetual-check-loss":
            if separator != ":" or loser not in {"black", "white"}:
                _fail("perpetual-check result lacks a valid loser color")
            winner_color = "white" if loser == "black" else "black"
            winner = "candidate" if winner_color == self.candidate_color else "handcrafted"
        elif separator:
            _fail("unexpected terminal suffix")
        elif reason in DRAW_REASONS:
            loser = None
            winner = None
        else:
            if self.side_to_move is None:
                _fail("decisive terminal reason requires the archived side to move")
            loser = self.side_to_move
            winner_color = "white" if loser == "black" else "black"
            winner = "candidate" if winner_color == self.candidate_color else "handcrafted"
        self.result = {"terminationReason": reason, "terminalLoserColor": loser,
                       "winner": winner, "raw": raw}
        return dict(self.result)

    def finish(self) -> dict[str, Any]:
        if not self.provenance_acked:
            _fail("missing provenance acknowledgement")
        if self.result is None:
            _fail("missing gameover result")
        return dict(self.result)


def _atomic_same(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        if path.read_bytes() != data:
            _fail(f"immutable output differs: {path}")
        return
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    except BaseException:
        try:
            os.unlink(temporary)
        except OSError:
            pass
        raise


def closeout(root: Path, required_outputs: Sequence[str], *,
             forbidden_dispatch: Mapping[str, Callable[[], Any]] | None = None,
             output_name: str = "closeout.json") -> dict[str, Any]:
    """Produce deterministic closeout solely from existing validated outputs.

    Dispatch callables are accepted only so tests/integrators can install
    raising spies.  They are intentionally never invoked.
    """
    root = Path(root).resolve()
    artifacts: list[dict[str, Any]] = []
    for relative in required_outputs:
        path = safe_reference(root, relative)
        if not path.is_file():
            _fail(f"missing required closeout output: {relative}")
        data = path.read_bytes()
        artifacts.append({"path": relative, "bytes": len(data), "sha256": digest_bytes(data)})
    value = {"schema": "haitaka-anhoku-strength-first-confirmation-closeout-v1",
             "status": "complete", "artifacts": artifacts,
             "dispatch": {name: 0 for name in sorted(forbidden_dispatch or {})}}
    destination = safe_reference(root, output_name)
    _atomic_same(destination, canonical_json(value))
    return value
