#!/usr/bin/env python3
"""Audit a partial Step 12 source output before strict resumable recovery."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re
import sys
from typing import Any


TRAIN_TOTAL_GAMES = 30000
VALIDATION_TOTAL_GAMES = 2016
SHARD_GAMES = 10
ENTRY_BYTES = 72
CONFIG_HASH = "f49bdf1d7d99bbb889a0bc516f4b3424647165afa88ad5fd745d58379935278e"
SEMANTIC_HASH = {
    "train": "a0f2dff06ca0a17630124f2945e3b2268c2248720f5d9799dbc62405a10b4d83",
    "validation": "a50632b2d95e45357a15bc270ea7d7aa4e2acd20c0bdceabcc123684be086142",
}
SHARD_RE = re.compile(r"^shard-(\d{6})\.(bin|json)$")
REBUILDABLE_BASELINE_FILES = {
    "source/automatic-parallelism-calibration-generated/artifacts/combined-generation-stage-v1.json",
    "source/automatic-parallelism-calibration-generated/datasets/train.bin",
    "source/automatic-parallelism-calibration-generated/datasets/train.json",
    "source/automatic-parallelism-calibration-generated/datasets/validation.bin",
    "source/automatic-parallelism-calibration-generated/datasets/validation.json",
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def error(errors: list[dict[str, Any]], kind: str, path: str, detail: str) -> None:
    errors.append({"kind": kind, "path": path, "detail": detail})


def audit_split(root: Path, split: str, total_games: int, errors: list[dict[str, Any]]) -> dict[str, Any]:
    directory = root / "source/automatic-parallelism-calibration-generated/datasets/shards" / split
    bins: dict[int, Path] = {}
    manifests: dict[int, Path] = {}
    if not directory.is_dir():
        error(errors, "missing-split-directory", str(directory), "split directory is missing")
        return {"split": split, "completePairs": 0, "shards": []}
    for path in directory.iterdir():
        if not path.is_file():
            continue
        match = SHARD_RE.match(path.name)
        if match is None:
            if path.name.endswith(".tmp"):
                error(errors, "temporary-file", str(path), "temporary shard file remains after the interrupted attempt")
            continue
        index = int(match.group(1))
        target = bins if match.group(2) == "bin" else manifests
        if index in target:
            error(errors, "duplicate-shard-file", str(path), "duplicate shard index")
        target[index] = path

    bin_ids = set(bins)
    manifest_ids = set(manifests)
    for index in sorted(bin_ids - manifest_ids):
        error(errors, "missing-sidecar", str(bins[index]), "payload has no JSON sidecar")
    for index in sorted(manifest_ids - bin_ids):
        error(errors, "missing-payload", str(manifests[index]), "JSON sidecar has no payload")

    shards: list[dict[str, Any]] = []
    for index in sorted(bin_ids & manifest_ids):
        payload = bins[index]
        sidecar = manifests[index]
        relative_payload = str(payload.relative_to(root))
        relative_sidecar = str(sidecar.relative_to(root))
        try:
            manifest = json.loads(sidecar.read_text(encoding="utf-8"))
        except (OSError, UnicodeError, json.JSONDecodeError) as exc:
            error(errors, "invalid-sidecar", relative_sidecar, str(exc))
            continue
        expected_start = index * SHARD_GAMES
        expected_count = min(SHARD_GAMES, total_games - expected_start)
        game_indices = [item.get("game_index") for item in manifest.get("games", [])]
        expected_indices = list(range(expected_start, expected_start + max(expected_count, 0)))
        checks = {
            "dataset": manifest.get("dataset") == split,
            "shardIndex": manifest.get("shard_index") == index,
            "gameStart": manifest.get("game_start") == expected_start,
            "gameCount": manifest.get("game_count") == expected_count,
            "gameListLength": len(game_indices) == expected_count,
            "gameCoverage": game_indices == expected_indices,
            "entryBytes": manifest.get("entry_bytes") == ENTRY_BYTES,
            "configHash": manifest.get("config_hash") == CONFIG_HASH,
            "semanticIdentity": manifest.get("generation_semantic_identity_sha256") == SEMANTIC_HASH[split],
            "noBootstrap": manifest.get("bootstrap_nnue") is None and manifest.get("bootstrap_nnue_sha256") is None,
            "buildMode": manifest.get("build_mode") == "anhoku+trajectory:handcrafted+label:handcrafted",
            "payloadLength": payload.stat().st_size == int(manifest.get("sampled_positions", -1)) * ENTRY_BYTES,
        }
        for name, passed in checks.items():
            if not passed:
                error(errors, "shard-integrity", relative_sidecar, f"{name} check failed")
        shards.append({
            "index": index,
            "payload": {"path": relative_payload, "bytes": payload.stat().st_size, "sha256": sha256(payload)},
            "sidecar": {"path": relative_sidecar, "bytes": sidecar.stat().st_size, "sha256": sha256(sidecar)},
            "gameStart": manifest.get("game_start"),
            "gameCount": manifest.get("game_count"),
            "sampledPositions": manifest.get("sampled_positions"),
            "checks": checks,
        })

    present = sorted(bin_ids & manifest_ids)
    gaps = [index for index in range(present[-1] + 1) if index not in set(present)] if present else []
    return {
        "split": split,
        "payloadFiles": len(bin_ids),
        "sidecarFiles": len(manifest_ids),
        "completePairs": len(bin_ids & manifest_ids),
        "presentShardIndices": present,
        "gapsThroughHighestPresent": gaps,
        "completedGamesInShardPairs": sum(int(item.get("gameCount") or 0) for item in shards),
        "sampledPositionsInShardPairs": sum(int(item.get("sampledPositions") or 0) for item in shards),
        "shards": shards,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    args = parser.parse_args()
    root = args.root.expanduser().resolve()
    errors: list[dict[str, Any]] = []
    output = root / "source/automatic-parallelism-calibration-generated"
    if not output.is_dir():
        error(errors, "missing-output", str(output), "operational output directory is missing")

    baseline_path = root / "preflight/recovery-baseline.json"
    baseline = json.loads(baseline_path.read_text(encoding="utf-8"))
    baseline_results: list[dict[str, Any]] = []
    for item in baseline.get("files", []):
        path = root / item["path"]
        if not path.is_file():
            error(errors, "baseline-file-missing", item["path"], "file from pre-recovery baseline is missing")
            baseline_results.append({"path": item["path"], "status": "missing"})
            continue
        actual = {"bytes": path.stat().st_size, "sha256": sha256(path)}
        status = "unchanged" if actual == {"bytes": item["bytes"], "sha256": item["sha256"]} else "changed"
        if status == "changed" and item["path"] in REBUILDABLE_BASELINE_FILES:
            status = "expected-rebuild"
        if status == "changed":
            error(errors, "baseline-file-changed", item["path"], "calibration baseline file changed during recovery sync")
        baseline_results.append({"path": item["path"], "expected": item, "actual": actual, "status": status})

    train = audit_split(root, "train", TRAIN_TOTAL_GAMES, errors)
    validation = audit_split(root, "validation", VALIDATION_TOTAL_GAMES, errors)
    aggregate: dict[str, Any] = {}
    for split in ("train", "validation"):
        path = output / "datasets" / f"{split}.json"
        if not path.is_file():
            aggregate[split] = {"status": "missing"}
            continue
        manifest = json.loads(path.read_text(encoding="utf-8"))
        pair_count = (train if split == "train" else validation)["completePairs"]
        aggregate[split] = {
            "status": "stale-partial-aggregate" if pair_count != int(manifest.get("shard_count", -1)) else "matches-pairs",
            "path": str(path.relative_to(root)),
            "bytes": path.stat().st_size,
            "sha256": sha256(path),
            "manifestShardCount": manifest.get("shard_count"),
            "manifestCompletedGames": manifest.get("completed_games"),
            "completeShardPairs": pair_count,
            "manifestJobs": manifest.get("jobs"),
        }

    complete = (
        not errors
        and train["completePairs"] == (TRAIN_TOTAL_GAMES + SHARD_GAMES - 1) // SHARD_GAMES
        and validation["completePairs"] == (VALIDATION_TOTAL_GAMES + SHARD_GAMES - 1) // SHARD_GAMES
        and all(item.get("status") == "matches-pairs" for item in aggregate.values())
    )
    receipt = {
        "schema": "haitaka-anhoku-strength-first-step12-recovery-inventory-v1",
        "experimentId": "strength-first-step12-independent-replication-20260916-v1",
        "status": "completed" if complete else "passed-partial-recovery" if not errors else "blocked-integrity-errors",
        "createdAt": datetime.now(timezone.utc).isoformat(),
        "hostSource": "192.168.0.12",
        "output": str(output.relative_to(root)),
        "configHash": CONFIG_HASH,
        "entryBytes": ENTRY_BYTES,
        "baseline": {
            "path": str(baseline_path.relative_to(root)),
            "status": (
                "all-unchanged"
                if all(item["status"] == "unchanged" for item in baseline_results)
                else "unchanged-except-expected-aggregate-rebuild"
                if all(item["status"] in {"unchanged", "expected-rebuild"} for item in baseline_results)
                else "mismatch"
            ),
            "fileCount": len(baseline_results),
            "rebuildableFiles": sorted(REBUILDABLE_BASELINE_FILES),
            "files": baseline_results,
        },
        "splits": {"train": train, "validation": validation},
        "aggregate": aggregate,
        "errors": errors,
        "resumePolicy": {
            "strictIdentity": True,
            "useJobsZero": True,
            "noIgnoreIdentityMismatch": True,
            "noNoResume": True,
            "noShardSelector": True,
            "staleAggregatesRebuiltByFullRun": True,
        },
        "completion": {
            "sourceGenerationComplete": complete,
            "expectedTrainShardPairs": (TRAIN_TOTAL_GAMES + SHARD_GAMES - 1) // SHARD_GAMES,
            "expectedValidationShardPairs": (VALIDATION_TOTAL_GAMES + SHARD_GAMES - 1) // SHARD_GAMES,
        },
    }
    receipt_path = root / "preflight/recovery-inventory.json"
    receipt_path.parent.mkdir(parents=True, exist_ok=True)
    receipt_path.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({
        "status": receipt["status"],
        "trainPairs": train["completePairs"],
        "validationPairs": validation["completePairs"],
        "errors": len(errors),
        "receipt": str(receipt_path),
    }, sort_keys=True))
    return 0 if not errors else 2


if __name__ == "__main__":
    raise SystemExit(main())
