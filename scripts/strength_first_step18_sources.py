#!/usr/bin/env python3
"""Safe-source and migration helpers for the Step 18 Section 9 repair.

This module only follows explicitly declared files.  In particular, it never
walks, stats, hashes, or opens the protected abandoned Step 9/10 outcome trees.
"""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import subprocess
from typing import Any, Callable, Iterable, Mapping, Sequence


PROTECTED_TOKENS = (
    "step9-independent-confirmation-20260913-abandoned-1000ms-partial-unopened",
    "step10-checkpoint-selection-20260914-v1/results",
    "step10-checkpoint-selection-20260914-v1/recovery",
)
STEP9_BINDINGS = {
    "contract100ms": ("r0/anhoku-strength-first/step9-confirmation-100ms-contract-v1.json", "f17469e029abec33bfe5fe47651395841921e88edca242c8edc1fd82c62295d4", None),
    "contract1000ms": ("r0/anhoku-strength-first/step9-confirmation-contract-v2.json", "b1e262c161d6fcf5965d8768aa2dcedbed62bb8225647fa3f550bf493cf88788", None),
    "generationA": ("r0/anhoku-strength-first/step9-confirmation-a-generation.json", "81c1e7ee5c81e0958e4d5db0fb3774b474fb7a7ebe8b20e67d9ed8a099be3b1a", None),
    "generationB": ("r0/anhoku-strength-first/step9-confirmation-b-generation.json", "c81d59e5639df92ae9f8132e722e9e3a14508b802fd05494cdc882c782e38559", None),
    "suiteA": ("haitaka_learn/openings/anhoku-strength-confirmation-a-v1.tsv", "d58c3adfdb7ba96f0cf5e21e36c97fe48d1d3df337293b92b33665744f497f39", 236018),
    "suiteB": ("haitaka_learn/openings/anhoku-strength-confirmation-b-v1.tsv", "4824a0c63bae39d86148cc1bc4bd6471b4c8f1296a460ec5b150fba0cce54578", 236127),
}
NORMALIZATION_DEPENDENCIES = (
    "Cargo.toml",
    "haitaka/Cargo.toml",
    "haitaka_learn/Cargo.toml",
    "haitaka_wasm/Cargo.toml",
    "haitaka_learn/src/step11_opening_gate.rs",
    "haitaka_learn/src/openings.rs",
    "haitaka_learn/src/dataset.rs",
    "haitaka_learn/src/step12_exclusion_inventory.rs",
    "haitaka/src/board/mod.rs",
    "haitaka/src/board/movegen/mod.rs",
    "haitaka/src/board/movegen/piece_moves.rs",
    "haitaka/src/board/parse.rs",
    "haitaka/src/board/validate.rs",
    "haitaka/src/board/zobrist.rs",
    "Cargo.lock",
)
HELPER_RELATIVE = "target/step18-preparation-helper/release/haitaka_learn"


class SourceError(RuntimeError):
    pass


def canonical_bytes(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":"),
                       ensure_ascii=False, allow_nan=False) + "\n").encode()


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def safe_path(repo: Path, relative: str) -> Path:
    if not isinstance(relative, str) or not relative or "\x00" in relative:
        raise SourceError("safe source path must be a nonempty string")
    if any(token in relative for token in PROTECTED_TOKENS):
        raise SourceError("protected outcome path is forbidden")
    candidate = (repo / relative).resolve()
    try:
        candidate.relative_to(repo.resolve())
    except ValueError as exc:
        raise SourceError("safe source escapes repository") from exc
    return candidate


def strict_json(path: Path) -> Any:
    def pairs(items: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in items:
            if key in result:
                raise SourceError("duplicate JSON key in %s" % path)
            result[key] = value
        return result
    try:
        return json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=pairs,
                          parse_constant=lambda value: (_ for _ in ()).throw(ValueError(value)))
    except (OSError, UnicodeError, ValueError, json.JSONDecodeError) as exc:
        raise SourceError("cannot read strict JSON %s: %s" % (path, exc)) from exc


def binding(repo: Path, relative: str, expected_hash: str | None = None,
            expected_bytes: int | None = None) -> dict[str, Any]:
    path = safe_path(repo, relative)
    if not path.is_file():
        raise SourceError("missing safe source %s" % relative)
    size, digest = path.stat().st_size, sha256_file(path)
    if expected_bytes is not None and size != expected_bytes:
        raise SourceError("safe source byte count differs: %s" % relative)
    if expected_hash is not None and digest != expected_hash:
        raise SourceError("safe source hash differs: %s" % relative)
    return {"path": relative, "bytes": size, "sha256": digest}


def text_rows(path: Path) -> int:
    rows = 0
    with path.open("r", encoding="utf-8") as stream:
        for raw in stream:
            line = raw.split("#", 1)[0].strip()
            if line:
                rows += 1
    return rows


def step9_identity(repo: Path, experiment_id: str) -> dict[str, Any]:
    files = {name: binding(repo, *spec) for name, spec in STEP9_BINDINGS.items()}
    c100 = strict_json(safe_path(repo, STEP9_BINDINGS["contract100ms"][0]))
    c1000 = strict_json(safe_path(repo, STEP9_BINDINGS["contract1000ms"][0]))
    ga = strict_json(safe_path(repo, STEP9_BINDINGS["generationA"][0]))
    gb = strict_json(safe_path(repo, STEP9_BINDINGS["generationB"][0]))
    if c100.get("supersedes", {}).get("sha256") != files["contract1000ms"]["sha256"]:
        raise SourceError("100ms contract does not bind the 1000ms contract")
    abandoned = c100.get("abandoned1000msCampaign", {}).get("path")
    if not isinstance(abandoned, str) or PROTECTED_TOKENS[0] not in abandoned:
        raise SourceError("abandoned campaign string binding differs")
    for label, contract_name, generation, suite in (("A", "generationA", ga, "suiteA"),
                                                     ("B", "generationB", gb, "suiteB")):
        declared = c1000.get("suites", {}).get(label, {})
        if declared.get("generationReceipt", {}).get("sha256") != files[contract_name]["sha256"]:
            raise SourceError("Step9 generation receipt link %s differs" % label)
        if declared.get("sha256") != files[suite]["sha256"]:
            raise SourceError("Step9 suite link %s differs" % label)
        if generation.get("output", {}).get("sha256") != files[suite]["sha256"]:
            raise SourceError("Step9 generation output %s differs" % label)
        if generation.get("positions") != 2048 or declared.get("openingSourceGroups") != 2048:
            raise SourceError("Step9 suite %s row declaration differs" % label)
        rows = text_rows(safe_path(repo, files[suite]["path"]))
        if rows != 2048:
            raise SourceError("Step9 suite %s requires 2048 data rows" % label)
        files[suite]["dataRows"] = rows
    return {
        "schema": "haitaka-step18-step9-safe-identity-v1", "status": "passed",
        "experimentId": experiment_id, "files": files,
        "relationships": {
            "replacementContractBindsAbandoned1000msContract": True,
            "abandonedCampaignPathString": abandoned,
            "generationReceiptsBindSuites": True,
            "completePreregisteredSuitesExcluded": ["anhoku-confirmation-a", "anhoku-confirmation-b"],
        },
        "protectedOutcomeTraversal": False,
    }


def normalization_binding(repo: Path, experiment_id: str) -> dict[str, Any]:
    dependencies = [binding(repo, item) for item in NORMALIZATION_DEPENDENCIES]
    helper = binding(repo, HELPER_RELATIVE)
    source = next(item for item in dependencies
                  if item["path"] == "haitaka_learn/src/step11_opening_gate.rs")
    return {
        "schema": "haitaka-step18-normalization-compatibility-v1", "status": "fresh-recompute-required",
        "experimentId": experiment_id,
        "definition": "lexicographic-min(board.to_string(), color_swap_anhoku_training_sfen(board.to_string()))",
        "familyDefinition": "repository-packed-board-reader-plus-canonical-color-swap-v1",
        "canonicalizationSource": source,
        "transitiveDependencies": [item for item in dependencies if item is not source],
        "lockedHelper": helper,
        "requiredTests": ["color_swap_boards_share_one_canonical_family",
                          "packed_inventory_reads_the_repository_record_shape",
                          "overlap-rejection-fixtures"],
        "oldWholeFileHashReuseAuthorized": False, "protectedOutcomeTraversal": False,
    }


def build_current_helper(repo: Path, root: Path, experiment_id: str,
                         runner: Callable[..., subprocess.CompletedProcess[str]] = subprocess.run) -> dict[str, Any]:
    """Test and build the CPU-only inventory helper from the bound current sources."""
    commands = [
        ["cargo", "test", "--locked", "--offline", "--release", "-p", "haitaka_learn",
         "--features", "anhoku", "step11_opening_gate", "--target-dir", "target/step18-preparation-helper"],
        ["cargo", "test", "--locked", "--offline", "--release", "-p", "haitaka_learn",
         "--features", "anhoku", "step12_exclusion_inventory", "--target-dir", "target/step18-preparation-helper"],
        ["cargo", "build", "--locked", "--offline", "--release", "-p", "haitaka_learn",
         "--bin", "haitaka_learn", "--features", "anhoku", "--target-dir", "target/step18-preparation-helper"],
    ]
    results = []
    for argv in commands:
        completed = runner(argv, cwd=str(repo), text=True, stdout=subprocess.PIPE,
                           stderr=subprocess.PIPE, timeout=3 * 60 * 60, check=False)
        results.append({"argv": argv, "exitCode": completed.returncode,
                        "stdoutSha256": sha256_bytes(completed.stdout.encode()),
                        "stderrSha256": sha256_bytes(completed.stderr.encode())})
        if completed.returncode != 0:
            raise SourceError("current canonical-family helper build/test failed: %s" % " ".join(argv))
    helper = binding(repo, HELPER_RELATIVE)
    dependencies = [binding(repo, item) for item in NORMALIZATION_DEPENDENCIES]
    return {"schema": "haitaka-step18-normalization-helper-build-v1", "status": "passed",
            "experimentId": experiment_id, "commands": results,
            "dependencies": dependencies, "binary": helper,
            "cpuOnly": True, "engineOrWasmRebuilt": False, "trainingDataGenerated": False,
            "protectedOutcomeTraversal": False}


def _inventory_metadata(path: Path) -> tuple[dict[str, Any], int]:
    lines = path.read_text(encoding="utf-8").splitlines()
    if not lines:
        raise SourceError("empty family inventory %s" % path)
    metadata = json.loads(lines[0])
    if metadata.get("kind") != "metadata":
        raise SourceError("family inventory metadata header missing: %s" % path)
    for line in lines[1:]:
        value = json.loads(line)
        if value.get("kind") != "family" or not isinstance(value.get("canonicalFamily"), str):
            raise SourceError("malformed family row in %s" % path)
    return metadata, len(lines) - 1


def verify_allowlist(repo: Path, allowlist_path: Path) -> list[dict[str, Any]]:
    allowlist = strict_json(allowlist_path)
    entries = allowlist.get("entries")
    if allowlist.get("status") != "frozen" or not isinstance(entries, list) or len(entries) != 31:
        raise SourceError("Step12 exclusion allowlist must contain 31 frozen entries")
    result = []
    base = allowlist_path.parent.parent
    for entry in entries:
        identifier = entry.get("id")
        payloads = entry.get("payload")
        inventory = entry.get("familyInventory")
        if not isinstance(identifier, str) or not isinstance(payloads, list) or not isinstance(inventory, Mapping):
            raise SourceError("malformed Step12 allowlist entry")
        verified_payloads = []
        for item in payloads:
            verified_payloads.append(binding(repo, item["path"], item.get("sha256"), item.get("bytes")))
        inventory_path = (base / inventory["path"]).resolve()
        if any(token in str(inventory_path) for token in PROTECTED_TOKENS):
            raise SourceError("protected inventory path")
        if sha256_file(inventory_path) != inventory.get("sha256"):
            raise SourceError("retained inventory hash differs: %s" % identifier)
        metadata, family_rows = _inventory_metadata(inventory_path)
        if family_rows != inventory.get("count") or metadata.get("rows") != inventory.get("rows"):
            raise SourceError("retained inventory row/header differs: %s" % identifier)
        result.append({"id": identifier, "payloads": verified_payloads,
                       "retainedInventory": {"path": str(inventory_path.relative_to(repo)),
                                             "bytes": inventory_path.stat().st_size,
                                             "sha256": sha256_file(inventory_path),
                                             "sourceRows": metadata["rows"], "familyRows": family_rows}})
    return result


def extract_schedule_tsv(schedule_path: Path, output: Path) -> dict[str, Any]:
    value = strict_json(schedule_path)
    rows = value.get("rows")
    if not isinstance(rows, list):
        raise SourceError("schedule rows missing")
    seen: set[tuple[str, str]] = set()
    values: list[tuple[str, str]] = []
    for row in rows:
        opening_id, sfen = row.get("openingId"), row.get("startSfen")
        if not isinstance(opening_id, str) or not isinstance(sfen, str):
            raise SourceError("schedule opening identity missing")
        key = (opening_id, sfen)
        if key not in seen:
            seen.add(key)
            values.append(key)
    data = "".join("%s\t%s\n" % item for item in values).encode()
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = output.with_name(".%s.%d.tmp" % (output.name, os.getpid()))
    temporary.write_bytes(data)
    os.replace(temporary, output)
    return {"source": {"path": str(schedule_path), "bytes": schedule_path.stat().st_size,
                       "sha256": sha256_file(schedule_path)},
            "output": {"path": str(output), "bytes": len(data), "sha256": sha256_bytes(data)},
            "uniqueOpeningRows": len(values)}


def run_inventory(helper: Path, payloads: Sequence[Path], output: Path, identifier: str,
                  runner: Callable[..., subprocess.CompletedProcess[str]] = subprocess.run) -> dict[str, Any]:
    argv = [str(helper), "step12-exclusion-inventory", "--output", str(output),
            "--inventory-id", identifier]
    for payload in payloads:
        if any(token in str(payload) for token in PROTECTED_TOKENS):
            raise SourceError("protected payload path")
        argv.extend(["--input", str(payload)])
    completed = runner(argv, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                       timeout=3 * 60 * 60, check=False)
    if completed.returncode != 0:
        raise SourceError("canonical-family helper failed for %s: %s" % (identifier, completed.stderr[-1000:]))
    receipt = json.loads(completed.stdout)
    if receipt.get("status") != "passed" or not output.is_file():
        raise SourceError("canonical-family helper produced no accepted inventory")
    return receipt


def source_inventory_plan(repo: Path, root: Path) -> dict[str, Any]:
    """Resolve every source by an explicit manifest/path; never discover recursively."""
    allowlist_path = repo / "out/anhoku-strength-first/step12-independent-replication-20260916-v1/preflight/exclusion-allowlist.json"
    baseline = verify_allowlist(repo, allowlist_path)
    by_id = {entry["id"]: entry for entry in baseline}
    for identifier, suite_name in (("anhoku-confirmation-a", "suiteA"),
                                   ("anhoku-confirmation-b", "suiteB")):
        entry = by_id.get(identifier)
        if (entry is None or len(entry["payloads"]) != 1 or
                entry["payloads"][0]["sha256"] != STEP9_BINDINGS[suite_name][1] or
                entry["retainedInventory"]["sourceRows"] != 2048 or
                entry["retainedInventory"]["familyRows"] != 2048):
            raise SourceError("Step12 allowlist does not retain the complete Step9 %s suite" % identifier)
    supplemental_specs = (
        ("step12-current-training", "out/anhoku-strength-first/step12-independent-replication-20260916-v1/data/frozen-corpus-v1/training.bin", "packed-corpus"),
        ("step12-current-validation", "out/anhoku-strength-first/step12-independent-replication-20260916-v1/data/frozen-corpus-v1/validation.bin", "packed-corpus"),
        ("step12-source-training", "out/anhoku-strength-first/step12-independent-replication-20260916-v1/source/automatic-parallelism-calibration-generated/datasets/train.bin", "packed-corpus"),
        ("step12-source-validation", "out/anhoku-strength-first/step12-independent-replication-20260916-v1/source/automatic-parallelism-calibration-generated/datasets/validation.bin", "packed-corpus"),
        ("step12-exposed-openings", "out/anhoku-strength-first/step12-independent-replication-20260916-v1/openings/development.tsv", "opening-suite"),
        ("step14-exposed-openings", "out/anhoku-strength-first/step14-browser-finalist-20260919-v1/openings/finalist.tsv", "opening-suite"),
    )
    supplemental = [{"id": identifier, "kind": kind,
                     "payloads": [binding(repo, relative)]}
                    for identifier, relative, kind in supplemental_specs]
    return {
        "schema": "haitaka-step18-source-inventory-plan-v1", "status": "resolved",
        "baselineAllowlist": binding(repo, str(allowlist_path.relative_to(repo))),
        "baselineEntries": baseline, "supplementalEntries": supplemental,
        "step13OpeningRelationship": {
            "newOpeningPopulation": False,
            "evidence": binding(repo, "out/anhoku-strength-first/step13-replication-diagnosis-20260919-v1/inputs.json"),
            "coveredBy": "step12-exposed-openings",
        },
        "recursiveDiscoveryUsed": False, "protectedOutcomeTraversal": False,
        "outputsDirectory": str((root / "preflight/current-families").resolve()),
    }


def recompute_source_freeze(repo: Path, root: Path, experiment_id: str,
                            runner: Callable[..., subprocess.CompletedProcess[str]] = subprocess.run) -> dict[str, Any]:
    plan = source_inventory_plan(repo, root)
    normalization = normalization_binding(repo, experiment_id)
    helper = safe_path(repo, normalization["lockedHelper"]["path"])
    rows: list[dict[str, Any]] = []
    all_entries = list(plan["baselineEntries"]) + list(plan["supplementalEntries"])
    union: set[str] = set()
    for entry in all_entries:
        identifier = entry["id"]
        payloads = [safe_path(repo, item["path"]) for item in entry["payloads"]]
        output = root / "preflight/current-families" / (identifier + ".jsonl")
        receipt = run_inventory(helper, payloads, output, identifier, runner=runner)
        metadata, family_count = _inventory_metadata(output)
        if metadata.get("canonicalizationSource", {}).get("sha256") != normalization["canonicalizationSource"]["sha256"]:
            raise SourceError("helper was not built from the current canonicalization source")
        if receipt.get("producer", {}).get("binary", {}).get("sha256") != normalization["lockedHelper"]["sha256"]:
            raise SourceError("inventory helper binary binding differs")
        for line in output.read_text(encoding="utf-8").splitlines()[1:]:
            union.add(json.loads(line)["canonicalFamily"])
        rows.append({"id": identifier, "payloads": entry["payloads"],
                     "inventory": {"path": str(output.resolve()), "bytes": output.stat().st_size,
                                   "sha256": sha256_file(output), "sourceRows": metadata["rows"],
                                   "canonicalFamilies": family_count},
                     "receipt": receipt})
    union_path = root / "preflight/current-families/union.jsonl"
    union_data = b"".join(canonical_bytes({"kind": "family", "canonicalFamily": item})
                          for item in sorted(union))
    union_path.parent.mkdir(parents=True, exist_ok=True)
    temporary = union_path.with_name(".%s.%d.tmp" % (union_path.name, os.getpid()))
    temporary.write_bytes(union_data)
    os.replace(temporary, union_path)
    return {
        "schema": "haitaka-step18-source-freeze-v1", "status": "passed",
        "experimentId": experiment_id, "planSha256": sha256_bytes(canonical_bytes(plan)),
        "normalizationSha256": sha256_bytes(canonical_bytes(normalization)),
        "entries": rows, "entryCount": len(rows),
        "union": {"path": str(union_path.resolve()), "bytes": len(union_data),
                  "sha256": sha256_bytes(union_data), "canonicalFamilies": len(union)},
        "membershipEvidence": {
            "selection": "required-before-selection-opening-acceptance",
            "holdout": "required-before-holdout-opening-acceptance",
        },
        "recursiveDiscoveryUsed": False, "protectedOutcomeTraversal": False,
    }
