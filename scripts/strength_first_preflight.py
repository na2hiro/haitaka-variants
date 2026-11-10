#!/usr/bin/env python3
"""Read-only Step 1 inventory and raw-game audit for the strength-first plan.

The preflight records the current source tree, validates the immutable R5
inputs that are being reused, and freezes a development schedule.  It does
not train, generate positions, contact a remote service, rent a GPU, or alter
the worktree.  The only optional write is the report requested with
``--output``.

The ``audit`` command deliberately derives its game totals from raw JSONL
records.  A report is an assertion to check, never an input to the totals.
This keeps a stale or hand-edited summary from looking like fresh evidence.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import subprocess
import sys
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any, Iterable


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONTRACT = Path("r0/anhoku-reboot/r5-fixed-data-exposure-contract.json")
DEFAULT_OPENINGS = Path("haitaka_learn/openings/anhoku-v3.tsv")
DEFAULT_R5_STATUS = Path("out/anhoku-reboot-r5-gpu/r5-phase-status.json")
DEFAULT_MODELS = {
    "c16": Path("out/anhoku-v0.6-phase7.1-preserved/lane-c-step-16.nnue"),
    "currentSeed17": Path("out/anhoku-reboot-r5-gpu/current-final.nnue"),
    "seed29": Path("out/anhoku-reboot-r5-gpu/replication-seed29/final.nnue"),
    "compact": Path("out/anhoku-reboot-r5-gpu/compact-final.nnue"),
}
# These are the immutable exported identities already recorded by the R5
# closeout.  They prevent a different same-sized file from being silently
# called "seed 17", "seed 29", or "compact" when the source receipt is not
# available.  CLI overrides can be used for a deliberately supplied fixture.
DEFAULT_MODEL_BINDINGS = {
    "c16": {"bytes": 161206531, "sha256": "049f72f3a3adcfeb260710264af6669da6346af35bd34092f6f6fa0ef531cfe0"},
    "currentSeed17": {"bytes": 161206489, "sha256": "7dc9af50a8618aa7a511aa891baeda8823015e2ade2a21d80346c77297be44d9"},
    "seed29": {"bytes": 161206489, "sha256": "67692b31bc86d3a71043cbfc176a96b8cc96cdb549ef0940c4f209a5afb044a4"},
    "compact": {"bytes": 83048665, "sha256": "99c86bbf86d81ed67dde341b06724be3f9e917cca16aa8e78dd2a5732fd0cac0"},
}
EXPECTED_SPLITS = {"training": 880_000, "validation": 60_000, "development": 60_000}
EXPECTED_OPENING_IDS = [f"anhoku-v3-{index:03d}" for index in range(1, 65)]
SHA256_LENGTH = 64
TERMINAL_REASONS = {
    "completed",
    "maximum-ply-capped",
    "no-legal-move",
    "repetition-draw",
    "perpetual-check-loss",
    "timeout",
    "resignation",
    "draw",
}


class PreflightError(RuntimeError):
    """Raised only for programmer/CLI failures; data failures are reported."""


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def canonical_sha256(value: Any) -> str:
    encoded = json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def json_load(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        raise
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise PreflightError(f"cannot parse JSON {path}: {exc}") from exc


def workspace_path(root: Path, value: str | Path) -> Path:
    path = Path(value)
    if path.is_absolute():
        return path
    return root / path


def _valid_hash(value: Any) -> bool:
    return (
        isinstance(value, str)
        and len(value) == SHA256_LENGTH
        and value == value.lower()
        and all(char in "0123456789abcdef" for char in value)
    )


def artifact_status(
    root: Path,
    binding: Any,
    label: str,
    *,
    path_override: str | Path | None = None,
) -> dict[str, Any]:
    """Inspect one immutable file without raising for missing/invalid data.

    ``status`` is intentionally one of ``valid``, ``missing``, or
    ``invalid``.  A malformed declaration is invalid; an absent declared file
    is missing.  This distinction is useful when a historical artifact was
    never retained versus when a retained artifact changed.
    """

    result: dict[str, Any] = {"label": label, "status": "invalid", "errors": []}
    if not isinstance(binding, dict):
        result["errors"].append("artifact declaration is not an object")
        return result
    declared_path = path_override if path_override is not None else binding.get("path")
    result["path"] = str(declared_path) if declared_path is not None else None
    if not isinstance(declared_path, (str, Path)) or not str(declared_path):
        result["errors"].append("path is missing")
        return result
    path = workspace_path(root, declared_path)
    result["absolutePath"] = str(path)
    expected_bytes = binding.get("bytes")
    expected_hash = binding.get("sha256")
    if isinstance(expected_bytes, bool) or not isinstance(expected_bytes, int) or expected_bytes < 0:
        result["errors"].append("bytes is not a non-negative integer")
    if not _valid_hash(expected_hash):
        result["errors"].append("sha256 is not lowercase hexadecimal SHA-256")
    if result["errors"]:
        return result
    try:
        if not path.is_file():
            result["status"] = "missing"
            result["errors"].append("file is absent")
            result["expectedBytes"] = expected_bytes
            result["expectedSha256"] = expected_hash
            return result
        actual_bytes = path.stat().st_size
        actual_hash = sha256_file(path)
    except FileNotFoundError:
        result["status"] = "missing"
        result["errors"].append("file is absent")
        return result
    except OSError as exc:
        result["errors"].append(f"cannot inspect file: {exc}")
        return result
    result.update(
        {
            "expectedBytes": expected_bytes,
            "expectedSha256": expected_hash,
            "actualBytes": actual_bytes,
            "actualSha256": actual_hash,
        }
    )
    if actual_bytes != expected_bytes:
        result["errors"].append(f"byte count {actual_bytes} != {expected_bytes}")
    if actual_hash != expected_hash:
        result["errors"].append(f"sha256 {actual_hash} != {expected_hash}")
    result["status"] = "valid" if not result["errors"] else "invalid"
    return result


def _status_from_checks(checks: Iterable[dict[str, Any]]) -> str:
    statuses = [item.get("status") for item in checks]
    if "invalid" in statuses:
        return "invalid"
    if "missing" in statuses:
        return "missing"
    return "valid"


def _git(root: Path, *args: str) -> tuple[str | None, str | None]:
    try:
        completed = subprocess.run(
            ["git", *args], cwd=root, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True
        )
    except (OSError, subprocess.CalledProcessError) as exc:
        detail = getattr(exc, "stderr", None) or str(exc)
        return None, detail.strip()
    return completed.stdout, None


def _porcelain_v2_paths(status: str) -> list[str]:
    """Extract paths from ``git status --porcelain=v2 -z`` records.

    Porcelain v2 uses a space-separated header followed by a path.  The
    path is the final field of ordinary (``1``), unmerged (``u``), and
    rename/copy (``2``) records; it is not the whole record.  A ``2`` record
    is followed by the original path as a second NUL-delimited record.  The
    previous implementation treated the complete header as a path, which
    made a dirty snapshot depend on git's metadata rather than the file name.
    """

    records = [record for record in status.split("\0") if record]
    paths: list[str] = []
    index = 0
    while index < len(records):
        record = records[index]
        kind = record[:1]
        if kind in {"?", "!"}:
            if len(record) > 2 and record[1] == " ":
                paths.append(record[2:])
            index += 1
            continue
        if kind == "1":
            fields = record.split(" ", 8)
            if len(fields) == 9 and fields[8]:
                paths.append(fields[8])
            index += 1
            continue
        if kind == "2":
            fields = record.split(" ", 9)
            if len(fields) == 10 and fields[9]:
                paths.append(fields[9])
            # With -z, the rename/copy source path is the next NUL-delimited
            # item and has no status header of its own.
            if index + 1 < len(records):
                paths.append(records[index + 1])
                index += 2
            else:
                index += 1
            continue
        if kind == "u":
            fields = record.split(" ", 10)
            if len(fields) == 11 and fields[10]:
                paths.append(fields[10])
            index += 1
            continue
        # Keep a conservative fallback for a future porcelain record type:
        # only accept a tab-delimited path, never the metadata header.
        if "\t" in record:
            paths.extend(field for field in record.split("\t")[1:] if field)
        index += 1
    return paths


def git_snapshot(root: Path) -> dict[str, Any]:
    """Capture commit and dirty paths; this function never writes to git."""

    commit, commit_error = _git(root, "rev-parse", "HEAD")
    status, status_error = _git(root, "status", "--porcelain=v2", "-z")
    result: dict[str, Any] = {
        "status": "valid" if commit and status_error is None else "invalid",
        "commit": commit.strip() if commit else None,
        "dirty": bool(status),
        "dirtyPaths": [],
        "porcelainV2": [],
        "errors": [],
    }
    if commit_error:
        result["errors"].append(f"rev-parse: {commit_error}")
    if status_error:
        result["errors"].append(f"status: {status_error}")
    if status is None:
        return result
    # Preserve the raw NUL-delimited records for evidence, while deriving a
    # clean path-only list for callers and reports.
    records = [record for record in status.split("\0") if record]
    result["porcelainV2"] = records
    result["dirtyPaths"] = sorted(set(_porcelain_v2_paths(status)))
    return result


def parse_openings(path: Path) -> tuple[dict[str, Any], list[dict[str, str]]]:
    """Parse the fixed v3 TSV and return a status plus ordered openings."""

    try:
        raw = path.read_bytes()
    except FileNotFoundError:
        return ({"status": "missing", "path": str(path), "errors": ["file is absent"]}, [])
    except OSError as exc:
        return ({"status": "invalid", "path": str(path), "errors": [str(exc)]}, [])
    status: dict[str, Any] = {
        "status": "valid",
        "path": str(path),
        "bytes": len(raw),
        "sha256": hashlib.sha256(raw).hexdigest(),
        "errors": [],
    }
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError as exc:
        status["status"] = "invalid"
        status["errors"].append(f"not UTF-8: {exc}")
        return status, []
    openings: list[dict[str, str]] = []
    for line_number, raw_line in enumerate(text.splitlines(), 1):
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        fields = raw_line.split("\t")
        if len(fields) != 2:
            status["errors"].append(f"line {line_number} does not have exactly two tab fields")
            continue
        opening_id, sfen = (field.strip() for field in fields)
        if not opening_id or not sfen:
            status["errors"].append(f"line {line_number} has an empty id or SFEN")
            continue
        openings.append({"id": opening_id, "sfen": sfen})
    if len(openings) != 64:
        status["errors"].append(f"expected 64 openings, found {len(openings)}")
    ids = [opening["id"] for opening in openings]
    if ids != EXPECTED_OPENING_IDS:
        status["errors"].append("opening ids/order are not anhoku-v3-001 through anhoku-v3-064")
    if len(set(ids)) != len(ids):
        status["errors"].append("opening ids are not unique")
    if status["errors"]:
        status["status"] = "invalid"
    return status, openings


def freeze_schedule(path: Path) -> dict[str, Any]:
    status, openings = parse_openings(path)
    result: dict[str, Any] = {
        "status": status["status"],
        "source": status,
        "pairCount": 128,
        "gameCount": 256,
        "repeatCount": 2,
        "gamesPerPair": 2,
        "openings": [],
        "pairs": [],
        "errors": list(status.get("errors", [])),
    }
    if status["status"] != "valid":
        return result
    for opening_index, opening in enumerate(openings):
        identity = f"{opening['id']}\t{opening['sfen']}\n".encode("utf-8")
        result["openings"].append(
            {
                "openingIndex": opening_index,
                "id": opening["id"],
                "sfen": opening["sfen"],
                "sfenSha256": hashlib.sha256(opening["sfen"].encode("utf-8")).hexdigest(),
                "identitySha256": hashlib.sha256(identity).hexdigest(),
            }
        )
    for repeat_index in range(2):
        for opening in result["openings"]:
            pair_index = repeat_index * 64 + opening["openingIndex"]
            games = [
                {"gameIndex": pair_index * 2, "aColor": "black", "bColor": "white"},
                {"gameIndex": pair_index * 2 + 1, "aColor": "white", "bColor": "black"},
            ]
            pair_identity = {
                "pairIndex": pair_index,
                "repeatIndex": repeat_index,
                "openingId": opening["id"],
                "openingIndex": opening["openingIndex"],
                "sfenSha256": opening["sfenSha256"],
                "games": games,
            }
            result["pairs"].append(
                {
                    **pair_identity,
                    "pairSha256": canonical_sha256(pair_identity),
                }
            )
    result["scheduleSha256"] = canonical_sha256(
        {
            "sourceSha256": status["sha256"],
            "repeatCount": 2,
            "pairs": result["pairs"],
        }
    )
    result["status"] = "valid"
    return result


def _binding_from_status(value: Any) -> dict[str, Any] | None:
    if not isinstance(value, dict):
        return None
    candidate = value.get("exportedInteger")
    if isinstance(candidate, dict) and "sha256" in candidate:
        return candidate
    candidate = value.get("exported", value)
    return candidate if isinstance(candidate, dict) and "sha256" in candidate else None


def _manifest_check(root: Path, manifest_check: dict[str, Any], split: str, contract: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    if manifest_check.get("status") != "valid":
        return errors
    manifest_path = workspace_path(root, manifest_check["path"])
    try:
        manifest = json_load(manifest_path)
    except (FileNotFoundError, PreflightError) as exc:
        errors.append(f"{split} manifest cannot be parsed: {exc}")
        return errors
    pilot = contract.get("pilot", {})
    expected_records = EXPECTED_SPLITS[split]
    checks = {
        "schema": "haitaka-r4-replacement-dataset-manifest-v1",
        "dataset": split,
        "records": expected_records,
        "entryBytes": pilot.get("entryBytes", 72),
        "contractSha256": pilot.get("labelContractSha256"),
        "labelEvaluator": pilot.get("labelEvaluator"),
        "labelBudget": pilot.get("labelBudget"),
        "targetSemantics": pilot.get("targetSemantics"),
        "scoreTransform": pilot.get("scoreTransform"),
        "resultLossEnabled": pilot.get("resultLossEnabled"),
        "unknownResultsIgnored": pilot.get("unknownResultsIgnored"),
    }
    for key, expected in checks.items():
        if expected is not None and manifest.get(key) != expected:
            errors.append(f"{split} manifest {key} mismatch")
    if not isinstance(manifest.get("binary"), dict) or not isinstance(manifest.get("sidecar"), dict):
        errors.append(f"{split} manifest lacks nested binary/sidecar identity")
    else:
        for part in ("binary", "sidecar"):
            declaration = manifest[part]
            declared = manifest_check.get(part, {})
            for key in ("path", "bytes", "sha256"):
                if declaration.get(key) != declared.get(key):
                    errors.append(f"{split} manifest {part}.{key} mismatch")
    return errors


def validate_contract_inputs(root: Path, contract_path: Path) -> dict[str, Any]:
    """Validate the contract-bound 1M payload and its authorization chain."""

    try:
        contract = json_load(contract_path)
    except FileNotFoundError:
        return {
            "status": "missing",
            "path": str(contract_path),
            "errors": ["contract is absent"],
            "artifacts": {},
            "splits": {},
        }
    except PreflightError as exc:
        return {
            "status": "invalid",
            "path": str(contract_path),
            "errors": [str(exc)],
            "artifacts": {},
            "splits": {},
        }
    contract_artifact = artifact_status(
        root,
        {"path": str(contract_path.relative_to(root)) if contract_path.is_relative_to(root) else str(contract_path), "bytes": contract_path.stat().st_size, "sha256": sha256_file(contract_path)},
        "R5 contract",
    )
    errors: list[str] = []
    if contract.get("schema") != "haitaka-anhoku-r5-fixed-data-exposure-contract-v1":
        errors.append("wrong R5 contract schema")
    if contract.get("phase") != "R5" or contract.get("ruleset") != "anhoku":
        errors.append("R5 contract phase/ruleset mismatch")
    if contract.get("status") != "frozen-for-preflight":
        errors.append("R5 contract is not frozen-for-preflight")
    pilot = contract.get("pilot")
    if not isinstance(pilot, dict) or pilot.get("records") != 1_000_000:
        errors.append("contract does not declare the 1M pilot")
    elif not isinstance(pilot.get("splits"), dict):
        errors.append("contract split declaration is malformed")
    authorization = contract.get("authorization", {})
    artifacts: dict[str, Any] = {"contract": contract_artifact}
    for key, label in (
        ("replacementR4Contract", "replacement R4 contract"),
        ("replacementR4Gate", "replacement R4 gate"),
        ("pilotManifest", "eligible 1M pilot manifest"),
    ):
        check = artifact_status(root, authorization.get(key), label)
        artifacts[key] = check
    # The static clone is the other immutable input to the historical R5
    # initialization lane.  It is useful context but does not change the
    # 1M split count.
    lanes = contract.get("initialization", {}).get("lanes", [])
    static = next((lane.get("staticClone") for lane in lanes if isinstance(lane, dict) and lane.get("staticClone")), None)
    if isinstance(static, dict):
        artifacts["staticCloneBinary"] = artifact_status(root, static.get("binary"), "static-clone binary")
        artifacts["staticCloneSidecar"] = artifact_status(root, static.get("sidecar"), "static-clone sidecar")
    splits: dict[str, Any] = {}
    split_declarations = pilot.get("splits") if isinstance(pilot, dict) else None
    if not isinstance(split_declarations, dict) or set(split_declarations) != set(EXPECTED_SPLITS):
        errors.append("contract splits are not exactly training/validation/development")
        split_declarations = {}
    for split, expected_records in EXPECTED_SPLITS.items():
        declaration = split_declarations.get(split)
        split_result: dict[str, Any] = {"records": expected_records, "artifacts": {}, "errors": []}
        if not isinstance(declaration, dict) or declaration.get("records") != expected_records:
            split_result["errors"].append(f"{split} record declaration mismatch")
            declaration = declaration if isinstance(declaration, dict) else {}
        for part in ("manifest", "binary", "sidecar"):
            check = artifact_status(root, declaration.get(part), f"{split} {part}")
            split_result["artifacts"][part] = check
        split_result["errors"].extend(_manifest_check(root, split_result["artifacts"], split, contract))
        binary = split_result["artifacts"]["binary"]
        entry_bytes = pilot.get("entryBytes", 72) if isinstance(pilot, dict) else 72
        if binary.get("status") == "valid" and binary.get("actualBytes") != expected_records * entry_bytes:
            split_result["errors"].append(f"{split} binary physical length mismatch")
        errors.extend(split_result["errors"])
        splits[split] = split_result
    # Verify the pilot manifest's own split counts and contract binding when
    # the manifest is available.  Individual artifact checks remain visible.
    pilot_check = artifacts.get("pilotManifest", {})
    if pilot_check.get("status") == "valid":
        try:
            pilot_manifest = json_load(workspace_path(root, pilot_check["path"]))
            if pilot_manifest.get("schema") != "haitaka-r4-replacement-eligible-pilot-v1":
                errors.append("pilot manifest schema mismatch")
            if pilot_manifest.get("eligibleForR5") is not True or pilot_manifest.get("records") != 1_000_000:
                errors.append("pilot manifest eligibility/record count mismatch")
            expected_contract = contract.get("authorization", {}).get("replacementR4Contract", {}).get("sha256")
            if expected_contract and pilot_manifest.get("contractSha256") != expected_contract:
                errors.append("pilot manifest replacement-contract binding mismatch")
            for split, count in EXPECTED_SPLITS.items():
                if pilot_manifest.get("splits", {}).get(split, {}).get("records") != count:
                    errors.append(f"pilot manifest {split} count mismatch")
                if pilot_manifest.get("splits", {}).get(split, {}).get("manifestSha256") != splits.get(split, {}).get("artifacts", {}).get("manifest", {}).get("expectedSha256"):
                    errors.append(f"pilot manifest {split} manifest hash mismatch")
        except (FileNotFoundError, PreflightError) as exc:
            errors.append(f"pilot manifest cannot be parsed: {exc}")
    # Bind the replacement gate where its JSON is retained.  This does not
    # reinterpret historical R5 results as a new authorization.
    gate_check = artifacts.get("replacementR4Gate", {})
    if gate_check.get("status") == "valid":
        try:
            gate = json_load(workspace_path(root, gate_check["path"]))
            if gate.get("schema") != "haitaka-anhoku-r4-replacement-gate-v8" or gate.get("passed") is not True or gate.get("r5Authorized") is not True:
                errors.append("replacement R4 gate is not a passing R5 input")
        except (FileNotFoundError, PreflightError) as exc:
            errors.append(f"replacement R4 gate cannot be parsed: {exc}")
    artifact_checks = list(artifacts.values()) + [part for split in splits.values() for part in split["artifacts"].values()]
    artifact_status_value = _status_from_checks(artifact_checks)
    if errors:
        artifact_status_value = "invalid" if artifact_status_value != "missing" or any("mismatch" in error or "schema" in error for error in errors) else artifact_status_value
    return {
        "status": artifact_status_value,
        "path": str(contract_path),
        "sha256": contract_artifact.get("actualSha256"),
        "contract": {
            "schema": contract.get("schema"),
            "phase": contract.get("phase"),
            "ruleset": contract.get("ruleset"),
        },
        "artifacts": artifacts,
        "splits": splits,
        "errors": errors,
    }


def model_inputs(
    root: Path,
    contract_result: dict[str, Any],
    r5_status_path: Path | None = None,
    overrides: dict[str, Path] | None = None,
) -> dict[str, Any]:
    bindings = {key: dict(value) for key, value in DEFAULT_MODEL_BINDINGS.items()}
    paths = dict(DEFAULT_MODELS)
    if overrides:
        paths.update(overrides)
    receipt_sources: dict[str, str] = {"c16": "R5 contract initialization.c16Baseline"}
    contract_sha = None
    try:
        contract = json_load(workspace_path(root, contract_result["path"]))
        c16 = contract.get("initialization", {}).get("c16Baseline")
        if isinstance(c16, dict):
            bindings["c16"] = dict(c16)
            paths["c16"] = Path(c16.get("path", paths["c16"]))
            receipt_sources["c16"] = "R5 contract initialization.c16Baseline"
        contract_sha = contract_result.get("sha256")
    except (FileNotFoundError, PreflightError, KeyError):
        pass
    # The selected seed-17 receipt is the strongest available identity source
    # for the current export.  The other two are read-only historical reports.
    status_path = r5_status_path or workspace_path(root, DEFAULT_R5_STATUS)
    try:
        status = json_load(status_path)
        selected = status.get("selectedCandidate", {})
        if isinstance(selected, dict) and isinstance(selected.get("exportedSha256"), str):
            bindings["currentSeed17"] = {"bytes": selected.get("exportedBytes"), "sha256": selected["exportedSha256"]}
            receipt_sources["currentSeed17"] = str(status_path)
    except (FileNotFoundError, PreflightError):
        pass
    # Keep actual user-supplied path overrides bound to their historical hash
    # only when a matching default identity was requested; this is primarily a
    # fixture affordance and still fails closed on a changed file.
    result: dict[str, Any] = {}
    for key in ("c16", "currentSeed17", "seed29", "compact"):
        binding = bindings[key]
        binding = {"path": str(paths[key]), **binding}
        check = artifact_status(root, binding, f"{key} integer export")
        check["identitySource"] = receipt_sources.get(key, "R5 retained export identity")
        if key == "compact":
            check["width"] = {"l1": 256, "l2": 16, "l3": 32}
        elif key != "c16":
            check["width"] = {"l1": 512, "l2": 16, "l3": 32}
        result[key] = check
    result["status"] = _status_from_checks(result.values())
    result["contractSha256"] = contract_sha
    return result


def jobs_inventory(root: Path, path: Path | None) -> dict[str, Any]:
    if path is None:
        return {
            "status": "not-supplied",
            "source": None,
            "local": [],
            "remote": [],
            "activeLocal": 0,
            "activeRemote": 0,
            "errors": [],
        }
    resolved = workspace_path(root, path)
    result: dict[str, Any] = {"source": str(path), "path": str(resolved), "errors": []}
    try:
        payload = json_load(resolved)
    except FileNotFoundError:
        result.update({"status": "missing", "local": [], "remote": [], "activeLocal": 0, "activeRemote": 0})
        result["errors"].append("inventory JSON is absent")
        return result
    except PreflightError as exc:
        result.update({"status": "invalid", "local": [], "remote": [], "activeLocal": 0, "activeRemote": 0})
        result["errors"].append(str(exc))
        return result
    if isinstance(payload, dict) and isinstance(payload.get("jobs"), dict):
        payload = payload["jobs"]
    if not isinstance(payload, dict):
        result.update({"status": "invalid", "local": [], "remote": [], "activeLocal": 0, "activeRemote": 0})
        result["errors"].append("inventory root must be an object")
        return result
    local = payload.get("local", payload.get("localJobs", []))
    remote = payload.get("remote", payload.get("remoteJobs", []))
    if not isinstance(local, list) or not isinstance(remote, list):
        result.update({"status": "invalid", "local": [], "remote": [], "activeLocal": 0, "activeRemote": 0})
        result["errors"].append("local and remote job collections must be arrays")
        return result
    result.update({"status": "valid", "local": local, "remote": remote, "payloadSha256": sha256_file(resolved)})
    active_words = {"active", "running", "queued", "starting", "loading", "provisioning", "pending", "created"}

    def active_count(items: list[Any]) -> int:
        count = 0
        for item in items:
            if isinstance(item, dict):
                value = item.get("status", item.get("state", item.get("actual_status", "")))
                if isinstance(value, str) and value.lower() in active_words:
                    count += 1
        return count

    result["activeLocal"] = active_count(local)
    result["activeRemote"] = active_count(remote)
    result["totalLocal"] = len(local)
    result["totalRemote"] = len(remote)
    return result


def build_preflight(
    root: Path = ROOT,
    *,
    contract_path: Path = DEFAULT_CONTRACT,
    openings_path: Path = DEFAULT_OPENINGS,
    jobs_path: Path | None = None,
    r5_status_path: Path | None = None,
    model_overrides: dict[str, Path] | None = None,
) -> dict[str, Any]:
    root = root.resolve()
    contract_path = workspace_path(root, contract_path)
    openings_path = workspace_path(root, openings_path)
    contract_result = validate_contract_inputs(root, contract_path)
    schedule = freeze_schedule(openings_path)
    models = model_inputs(root, contract_result, r5_status_path, model_overrides)
    jobs = jobs_inventory(root, jobs_path)
    git = git_snapshot(root)
    required_checks = [contract_result, schedule, models]
    errors = list(contract_result.get("errors", [])) + list(schedule.get("errors", []))
    for key, check in models.items():
        if isinstance(check, dict) and check.get("status") in {"missing", "invalid"}:
            errors.extend(f"{key}: {error}" for error in check.get("errors", []))
    if git.get("status") != "valid":
        errors.extend(git.get("errors", []))
    overall_status = _status_from_checks(required_checks)
    if errors:
        overall_status = "invalid" if overall_status != "missing" or any("mismatch" in error or "schema" in error for error in errors) else overall_status
    return {
        "schema": "haitaka-anhoku-strength-first-preflight-v1",
        "status": overall_status,
        "readOnly": True,
        "root": str(root),
        "git": git,
        "contract": contract_result,
        "models": models,
        "developmentSchedule": schedule,
        "jobInventory": jobs,
        "checks": {
            "r5OneMillionSplits": contract_result.get("status"),
            "integerExports": models.get("status"),
            "anhokuV3Schedule": schedule.get("status"),
            "activeJobInventory": jobs.get("status"),
        },
        "trainingStarted": False,
        "gpuRentalRequested": False,
        "gpuRentalAuthorized": False,
        "readyForStep1": overall_status == "valid",
        "errors": errors,
        "sideEffects": [],
        "note": "Existing historical artifacts are inspected in place and are not copied, renamed, or deleted.",
    }


def _as_nonnegative_int(value: Any, label: str, errors: list[str]) -> int | None:
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(float(value)) or int(value) != value or value < 0:
        errors.append(f"{label} is not a non-negative integer")
        return None
    return int(value)


def _first_value(row: dict[str, Any], names: tuple[str, ...], default: Any = None) -> Any:
    for name in names:
        if name in row:
            return row[name]
    return default


def _result_score(row: dict[str, Any], errors: list[str], line_number: int) -> tuple[str | None, float | None]:
    result = row.get("result")
    winner = row.get("winner")
    normalized_result: str | None = None
    if isinstance(result, str):
        value = result.strip().lower().replace("_", "-")
        if value in {"a-win", "awin", "a"}:
            normalized_result = "a-win"
        elif value in {"b-win", " bwin", "bwin", "b"}:
            normalized_result = "b-win"
        # Native self-play preserves maximum-ply adjudications as ``capped``
        # while scoring them as half a point in the summary.  Keep that raw
        # status available through terminationReason, but normalize its score
        # here so capped games are retained in their color-swapped pairs.
        elif value in {"draw", "drawn", "tie", "0.5-0.5", "capped"}:
            normalized_result = "draw"
    if normalized_result is None and winner is not None:
        if isinstance(winner, str) and winner.upper() == "A":
            normalized_result = "a-win"
        elif isinstance(winner, str) and winner.upper() == "B":
            normalized_result = "b-win"
        elif winner is None:
            normalized_result = "draw"
    if normalized_result is None:
        errors.append(f"line {line_number}: result/winner is not a supported outcome")
        return None, None
    expected_winner = {"a-win": "A", "b-win": "B", "draw": None}[normalized_result]
    if winner is not None and winner != expected_winner and not (isinstance(winner, str) and winner.upper() == expected_winner):
        errors.append(f"line {line_number}: result and winner disagree")
    score = {"a-win": 1.0, "b-win": 0.0, "draw": 0.5}[normalized_result]
    if "scoreA" in row:
        try:
            supplied = float(row["scoreA"])
            if not math.isfinite(supplied) or supplied != score:
                errors.append(f"line {line_number}: scoreA disagrees with result")
        except (TypeError, ValueError):
            errors.append(f"line {line_number}: scoreA is not numeric")
    return normalized_result, score


def _game_counter(
    row: dict[str, Any],
    search_field_names: tuple[str, ...],
    aggregate_field_names: tuple[str, ...],
    searches: list[Any],
    line_number: int,
    errors: list[str],
) -> int:
    """Count one game-level fault without mixing search and aggregate shapes.

    Browser/raw harnesses may retain one boolean per search while the native
    self-play JSONL stores only a per-game aggregate (``fallbacks`` or
    ``missingMoves``).  Prefer the detailed searches whenever they exist;
    consult only the explicitly named aggregate fields when they do not.
    """
    if searches:
        total = 0
        for trace in searches:
            if not isinstance(trace, dict):
                errors.append(f"line {line_number}: search trace is not an object")
                continue
            for name in search_field_names:
                if trace.get(name) is True:
                    total += 1
                    break
        return total
    supplied_name = next((name for name in aggregate_field_names if name in row), None)
    if supplied_name is not None:
        value = _as_nonnegative_int(row[supplied_name], f"line {line_number} {supplied_name}", errors)
        return value or 0
    failure = row.get("failureState")
    if failure not in (None, False, ""):
        # A failure state without a more precise trace is itself one failed
        # search.  This is conservative for old native JSONL records.
        return 1
    return 0


def _game_nodes_depth(row: dict[str, Any], line_number: int, errors: list[str]) -> tuple[int, int, int, int | None]:
    total_value = _first_value(row, ("totalNodes", "alphaBetaNodes"), None)
    total = _as_nonnegative_int(total_value, f"line {line_number} totalNodes", errors)
    alpha = _as_nonnegative_int(row.get("alphaBetaNodes", total), f"line {line_number} alphaBetaNodes", errors)
    qnodes = _as_nonnegative_int(row.get("qnodes", 0), f"line {line_number} qnodes", errors)
    if total is None:
        total = alpha or 0
    if alpha is None:
        alpha = total
    if "totalNodes" in row and "alphaBetaNodes" in row and total != alpha:
        errors.append(f"line {line_number}: totalNodes and alphaBetaNodes disagree")
    depth_value = row.get("completedDepth")
    if depth_value is None:
        depths = []
        for side in ("aBreakdown", "bBreakdown"):
            if isinstance(row.get(side), dict) and row[side].get("completedDepth") is not None:
                depths.append(row[side]["completedDepth"])
        depth_value = max(depths) if depths else 0
    depth = _as_nonnegative_int(depth_value, f"line {line_number} completedDepth", errors)
    return total, alpha, qnodes or 0, depth


def _termination_reason(row: dict[str, Any], line_number: int, errors: list[str]) -> str:
    reason = row.get("terminationReason", row.get("termination_reason"))
    if not isinstance(reason, str) or not reason.strip():
        errors.append(f"line {line_number}: terminationReason is missing")
        return "missing"
    reason = reason.strip()
    return reason


def _compare_value(expected: Any, actual: Any, path: str, mismatches: list[str]) -> None:
    if expected != actual:
        mismatches.append(f"{path}: report={expected!r}, recomputed={actual!r}")


def _check_report(report_path: Path, recomputed: dict[str, Any]) -> dict[str, Any]:
    result: dict[str, Any] = {"status": "valid", "path": str(report_path), "mismatches": [], "errors": []}
    try:
        report = json_load(report_path)
    except FileNotFoundError:
        result.update({"status": "missing"})
        result["errors"].append("report is absent")
        return result
    except PreflightError as exc:
        result.update({"status": "invalid"})
        result["errors"].append(str(exc))
        return result
    if not isinstance(report, dict):
        result.update({"status": "invalid"})
        result["errors"].append("report root is not an object")
        return result
    summary = report.get("summary", report)
    if not isinstance(summary, dict):
        result.update({"status": "invalid"})
        result["errors"].append("report summary is not an object")
        return result
    mappings = {
        "games": "games",
        "aWins": "aWins",
        "bWins": "bWins",
        "draws": "draws",
        "pairCount": "pairCount",
        "pairScoreBins": "pairScoreBins",
        "fallbacks": "fallbacks",
        "missingMoves": "missingMoves",
        "totalNodes": "totalNodes",
        "alphaBetaNodes": "alphaBetaNodes",
        "qnodes": "qnodes",
        "completedDepth": "completedDepth",
        "terminationReasons": "terminationReasons",
        "terminationTotals": "terminationReasons",
    }
    for report_key, expected_key in mappings.items():
        if report_key in summary:
            _compare_value(summary[report_key], recomputed[expected_key], f"summary.{report_key}", result["mismatches"])
    if "decidedGames" in summary:
        _compare_value(summary["decidedGames"], recomputed["aWins"] + recomputed["bWins"], "summary.decidedGames", result["mismatches"])
    result["reportSha256"] = sha256_file(report_path)
    if result["mismatches"]:
        result["status"] = "invalid"
    return result


def audit_raw_games(games_path: Path, report_path: Path | None = None) -> dict[str, Any]:
    """Recompute an aggregate from raw self-play JSONL and check its report."""

    try:
        raw_bytes = games_path.stat().st_size
        raw_hash = sha256_file(games_path)
    except FileNotFoundError:
        return {"schema": "haitaka-anhoku-strength-first-game-audit-v1", "status": "missing", "path": str(games_path), "errors": ["raw JSONL is absent"], "passed": False}
    except OSError as exc:
        return {"schema": "haitaka-anhoku-strength-first-game-audit-v1", "status": "invalid", "path": str(games_path), "errors": [str(exc)], "passed": False}
    rows: list[dict[str, Any]] = []
    errors: list[str] = []
    try:
        with games_path.open("r", encoding="utf-8") as stream:
            for line_number, line in enumerate(stream, 1):
                if not line.strip():
                    errors.append(f"line {line_number}: blank line")
                    continue
                try:
                    value = json.loads(line)
                except (UnicodeDecodeError, json.JSONDecodeError) as exc:
                    errors.append(f"line {line_number}: invalid JSON: {exc}")
                    continue
                if not isinstance(value, dict):
                    errors.append(f"line {line_number}: JSONL row is not an object")
                    continue
                rows.append(value)
    except (OSError, UnicodeDecodeError) as exc:
        errors.append(f"cannot read raw JSONL: {exc}")
    pair_rows: dict[int, list[dict[str, Any]]] = defaultdict(list)
    seen_games: set[int] = set()
    a_wins = b_wins = draws = fallback_total = missing_total = 0
    total_nodes = alpha_nodes = qnodes_total = depth_total = 0
    depth_values: list[int] = []
    termination_counts: Counter[str] = Counter()
    game_rows: list[dict[str, Any]] = []
    for index, row in enumerate(rows, 1):
        row_errors: list[str] = []
        pair_index = _as_nonnegative_int(row.get("pairIndex"), f"line {index} pairIndex", row_errors)
        game_index = _as_nonnegative_int(row.get("gameIndex"), f"line {index} gameIndex", row_errors)
        if game_index is not None:
            if game_index in seen_games:
                row_errors.append(f"line {index}: duplicate gameIndex {game_index}")
            seen_games.add(game_index)
        result, score = _result_score(row, row_errors, index)
        reason = _termination_reason(row, index, row_errors)
        searches = row.get("searches") if isinstance(row.get("searches"), list) else []
        fallbacks = _game_counter(
            row,
            ("emergencyFallbackUsed", "emergency_fallback_used", "fallbackUsed", "fallback_used"),
            ("fallbacks", "emergencyFallbacks", "fallbackCount", "emergencyFallbackCount"),
            searches,
            index,
            row_errors,
        )
        missing = _game_counter(
            row,
            ("missingMove", "missing_move"),
            ("missingMoves", "missingMoveCount", "missingCount"),
            searches,
            index,
            row_errors,
        )
        total, alpha, qnodes, depth = _game_nodes_depth(row, index, row_errors)
        plies = row.get("plies")
        moves = row.get("moves")
        if plies is not None:
            actual_plies = _as_nonnegative_int(plies, f"line {index} plies", row_errors)
            if actual_plies is not None and isinstance(moves, list) and actual_plies != len(moves):
                row_errors.append(f"line {index}: plies does not equal moves length")
        if pair_index is not None:
            pair_rows[pair_index].append(row)
        if result is not None:
            if result == "a-win":
                a_wins += 1
            elif result == "b-win":
                b_wins += 1
            else:
                draws += 1
        fallback_total += fallbacks
        missing_total += missing
        total_nodes += total
        alpha_nodes += alpha
        qnodes_total += qnodes
        if depth is not None:
            depth_total += depth
            depth_values.append(depth)
        termination_counts[reason] += 1
        errors.extend(row_errors)
        game_rows.append(
            {
                "line": index,
                "pairIndex": pair_index,
                "gameIndex": game_index,
                "scoreA": score,
                "result": result,
                "terminationReason": reason,
                "fallbacks": fallbacks,
                "missingMoves": missing,
                "totalNodes": total,
                "alphaBetaNodes": alpha,
                "qnodes": qnodes,
                "completedDepth": depth,
            }
        )
    bins = [0, 0, 0, 0, 0]
    pair_errors: list[str] = []
    for pair_index, pair in sorted(pair_rows.items()):
        if len(pair) != 2:
            pair_errors.append(f"pair {pair_index} has {len(pair)} games; expected two color-swapped games")
            continue
        scored = []
        colors: set[str] = set()
        sfens: set[str] = set()
        for row in pair:
            result, score = _result_score(row, [], 0)
            if score is not None:
                scored.append(score)
            color = row.get("aColor")
            if isinstance(color, str):
                colors.add(color.lower())
            sfen = row.get("startSfen")
            if isinstance(sfen, str):
                sfens.add(sfen)
        if len(scored) != 2:
            continue
        if colors and colors != {"black", "white"}:
            pair_errors.append(f"pair {pair_index} does not swap A's colors")
        if len(sfens) > 1:
            pair_errors.append(f"pair {pair_index} has different start SFENs")
        total_score = sum(scored)
        bin_index = int(round(total_score * 2))
        if bin_index not in range(5):
            pair_errors.append(f"pair {pair_index} score is outside [0,2]")
        else:
            bins[bin_index] += 1
    errors.extend(pair_errors)
    complete_pairs = sum(bins)
    recomputed: dict[str, Any] = {
        "games": len(rows),
        "aWins": a_wins,
        "bWins": b_wins,
        "draws": draws,
        "decidedGames": a_wins + b_wins,
        "aScore": a_wins + draws * 0.5,
        "pairCount": complete_pairs,
        "pairScoreBins": bins,
        "fallbacks": fallback_total,
        "missingMoves": missing_total,
        "terminationReasons": dict(sorted(termination_counts.items())),
        "totalNodes": total_nodes,
        "alphaBetaNodes": alpha_nodes,
        "qnodes": qnodes_total,
        "combinedNodes": alpha_nodes + qnodes_total,
        "completedDepth": max(depth_values) if depth_values else 0,
        "minCompletedDepth": min(depth_values) if depth_values else 0,
        "maxCompletedDepth": max(depth_values) if depth_values else 0,
        "completedDepthTotal": depth_total,
        "averageCompletedDepth": depth_total / len(depth_values) if depth_values else 0.0,
    }
    report_check = _check_report(report_path, recomputed) if report_path is not None else {"status": "not-supplied", "mismatches": [], "errors": []}
    status = "invalid" if errors or report_check.get("status") == "invalid" else "valid"
    result = {
        "schema": "haitaka-anhoku-strength-first-game-audit-v1",
        "status": status,
        "passed": status == "valid" and report_check.get("status") in {"valid", "not-supplied"},
        "rawGames": {"path": str(games_path), "bytes": raw_bytes, "sha256": raw_hash},
        "recomputed": recomputed,
        "terminationTotals": recomputed["terminationReasons"],
        "games": game_rows,
        "reportCheck": report_check,
        "errors": errors,
        "independentFromReport": True,
    }
    return result


# Public aliases make the small module convenient to use from focused tests
# and from a later paired harness without tying that harness to CLI names.
audit_self_play = audit_raw_games
audit_games = audit_raw_games
build_schedule = freeze_schedule


def _write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def _resolve_cli_path(root: Path, value: str | None) -> Path | None:
    return workspace_path(root, value) if value is not None else None


def _cli() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    preflight = subparsers.add_parser("preflight", help="inventory immutable inputs and freeze the v3 schedule")
    preflight.add_argument("--workspace", type=Path, default=ROOT)
    preflight.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)
    preflight.add_argument("--openings", type=Path, default=DEFAULT_OPENINGS)
    preflight.add_argument("--jobs-json", type=Path, default=None)
    preflight.add_argument("--r5-status", type=Path, default=DEFAULT_R5_STATUS)
    preflight.add_argument("--current-seed17", type=Path, default=None)
    preflight.add_argument("--seed29", type=Path, default=None)
    preflight.add_argument("--compact", type=Path, default=None)
    preflight.add_argument("--output", type=Path, default=None)
    audit = subparsers.add_parser("audit", help="recompute totals from raw self-play JSONL")
    audit.add_argument("--games", type=Path, required=True)
    audit.add_argument("--report", type=Path, default=None)
    audit.add_argument("--output", type=Path, default=None)
    args = parser.parse_args()
    if args.command == "preflight":
        root = args.workspace.resolve()
        overrides = {
            key: path
            for key, path in (
                ("currentSeed17", args.current_seed17),
                ("seed29", args.seed29),
                ("compact", args.compact),
            )
            if path is not None
        }
        report = build_preflight(
            root,
            contract_path=args.contract,
            openings_path=args.openings,
            jobs_path=args.jobs_json,
            r5_status_path=args.r5_status,
            model_overrides=overrides,
        )
    else:
        games = args.games.resolve()
        report = audit_raw_games(games, args.report.resolve() if args.report else None)
    rendered = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.output is not None:
        output = args.output.resolve()
        _write_json(output, report)
        print(output)
    else:
        print(rendered, end="")
    return 0 if report.get("passed", report.get("readyForStep1", False)) else 1


if __name__ == "__main__":
    raise SystemExit(_cli())
