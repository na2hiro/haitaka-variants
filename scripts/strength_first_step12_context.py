#!/usr/bin/env python3
"""Hash-bound Step 12 trainer-context assembly shared by fixture and backend."""

from __future__ import annotations

from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
from typing import Any


TRAINER_REVISION = "61666d9e3653e4df9881b14c23f8fdcc4bf7779b"
ASSEMBLY_SCHEMA = "haitaka-anhoku-strength-first-step12-training-context-assembly-v1"
PATCHES = (
    "trainer-patches/variant-nnue-pytorch-phase7.1.patch",
    "trainer-patches/variant-nnue-pytorch-r5.patch",
)
OVERLAYS = (
    "haitaka_learn/trainer_overlay/feature_set.py",
    "haitaka_learn/trainer_overlay/features.py",
    "haitaka_learn/trainer_overlay/donor_features.py",
    "haitaka_learn/trainer_overlay/training_data_loader.cpp",
)
CONTEXT_COPIES = ("trainer-patches/evaluate.py",)
CUDA_CONTEXT_COPIES = (
    "scripts/strength_first_step12_cuda_backward.py",
    "scripts/strength_first_step12_cuda_backward.cu",
    "scripts/strength_first_step12_cuda_backward_reference.py",
    "scripts/strength_first_step12_cuda_runtime.py",
    "scripts/strength_first_step12_cuda_backward_tests.py",
)
SOURCE_FILES = (
    "model.py",
    "train.py",
    "serialize.py",
    "ranger.py",
    "feature_transformer.py",
    "nnue_dataset.py",
    "variant.py",
    "variant.h",
    "evaluate.py",
    "feature_set.py",
    "features.py",
    "donor_features.py",
    "training_data_loader.cpp",
)


class ContextAssemblyError(RuntimeError):
    pass


def now() -> str:
    return datetime.now(timezone.utc).isoformat()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def artifact(path: Path) -> dict[str, Any]:
    if path.is_symlink() or not path.is_file():
        raise ContextAssemblyError(f"missing or symlinked context artifact: {path}")
    return {"path": str(path), "bytes": path.stat().st_size, "sha256": sha256(path)}


def atomic_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temporary.write_bytes(json.dumps(value, indent=2, sort_keys=True).encode("utf-8") + b"\n")
    os.replace(temporary, path)


def _command(argv: list[str], cwd: Path, *, capture: bool = True) -> dict[str, Any]:
    completed = subprocess.run(argv, cwd=cwd, capture_output=capture, text=True, check=False)
    return {
        "argv": [str(item) for item in argv],
        "cwd": str(cwd),
        "returncode": completed.returncode,
        "stdout": (completed.stdout or "")[-8000:] if capture else "",
        "stderr": (completed.stderr or "")[-8000:] if capture else "",
    }


def _require_success(record: dict[str, Any], label: str) -> None:
    if record["returncode"] != 0:
        raise ContextAssemblyError(f"{label} failed with exit {record['returncode']}: {record['stderr'] or record['stdout']}")


def _reject_files(context: Path) -> list[str]:
    return sorted(str(path.relative_to(context)) for path in context.rglob("*") if path.is_file() and path.suffix == ".rej")


def _backup_files(context: Path) -> list[Path]:
    return sorted(path for path in context.rglob("*") if path.is_file() and path.suffix == ".orig")


def _source_hashes(context: Path) -> dict[str, dict[str, Any]]:
    return {name: artifact(context / name) for name in SOURCE_FILES}


def _source_identities(source_hashes: dict[str, dict[str, Any]]) -> dict[str, dict[str, Any]]:
    return {name: {"bytes": value["bytes"], "sha256": value["sha256"]} for name, value in source_hashes.items()}


def context_receipt_path(root: Path, purpose: str) -> Path:
    if purpose == "cpu-fixture":
        return root / "preflight" / "training-context-assembly.json"
    safe = re.sub(r"[^A-Za-z0-9_.-]+", "-", purpose).strip("-") or "context"
    return root / "preflight" / "deterministic-cuda-v1" / f"context-assembly-{safe}.json"


def _assemble_tree(
    *,
    repo_root: Path,
    external: Path,
    patch_executable: Path,
    width: dict[str, int],
    r5: Any,
    purpose: str,
) -> tuple[tempfile.TemporaryDirectory[str], Path, dict[str, Any]]:
    revision = subprocess.check_output(["git", "-C", str(external), "rev-parse", "HEAD"], text=True).strip()
    if revision != TRAINER_REVISION:
        raise ContextAssemblyError(f"pinned checkout revision differs: {revision}")
    if patch_executable.is_symlink() or not patch_executable.is_file() or not os.access(patch_executable, os.X_OK):
        raise ContextAssemblyError(f"bound patch executable is unavailable or not executable: {patch_executable}")

    temporary = tempfile.TemporaryDirectory(prefix=f"haitaka-step12-{purpose}-")
    base = Path(temporary.name)
    context = base / "context"
    context.mkdir()
    archive = base / "trainer.tar"
    with archive.open("wb") as stream:
        subprocess.run(["git", "-C", str(external), "archive", "--format=tar", revision], stdout=stream, check=True)
    assembly: dict[str, Any] = {
        "purpose": purpose,
        "revision": revision,
        "archive": artifact(archive),
        "patchExecutable": artifact(patch_executable),
        "patchVersion": subprocess.check_output([str(patch_executable), "--version"], text=True).splitlines()[0],
        "patches": [],
        "copiedFiles": [],
    }
    subprocess.run(["tar", "-xf", str(archive), "-C", str(context)], check=True)
    for relative in PATCHES:
        patch_path = repo_root / relative
        patch_artifact = artifact(patch_path)
        dry_run = [str(patch_executable), "--batch", "--forward", "--dry-run", "--fuzz=3", "-p1", "-i", str(patch_path)]
        apply = [str(patch_executable), "--batch", "--forward", "--fuzz=3", "-p1", "-i", str(patch_path)]
        dry_record = _command(dry_run, context)
        _require_success(dry_record, f"dry-run {relative}")
        apply_record = _command(apply, context)
        _require_success(apply_record, f"apply {relative}")
        backups = _backup_files(context)
        assembly["patches"].append({"input": patch_artifact, "dryRun": dry_record, "apply": apply_record, "disposableBackupsRemoved": [str(path.relative_to(context)) for path in backups]})
        rejects = _reject_files(context)
        if rejects:
            raise ContextAssemblyError(f"patch application left reject files: {rejects}")
        for backup in backups:
            backup.unlink()

    for relative in OVERLAYS + CONTEXT_COPIES + CUDA_CONTEXT_COPIES:
        source = repo_root / relative
        destination = context / Path(relative).name
        shutil.copy2(source, destination)
        assembly["copiedFiles"].append({"input": artifact(source), "output": artifact(destination)})
    r5.write_variant(context)
    os.environ.update({"HAITAKA_NNUE_L1": str(width["l1"]), "HAITAKA_NNUE_L2": str(width["l2"]), "HAITAKA_NNUE_L3": str(width["l3"])})
    assembly["width"] = width
    assembly["sourceHashes"] = _source_hashes(context)
    return temporary, context, assembly


def prepare_context(
    *,
    repo_root: Path,
    root: Path,
    external: Path,
    patch_binding: dict[str, Any],
    width: dict[str, int],
    r5: Any,
    cmake: str,
    purpose: str,
) -> tuple[tempfile.TemporaryDirectory[str], Path]:
    """Assemble, independently replay, receipt, then build one context."""
    patch_executable = Path(patch_binding["path"]).resolve()
    if patch_executable.is_symlink() or not patch_executable.is_file():
        raise ContextAssemblyError(f"bound patch executable is missing: {patch_executable}")
    if patch_executable.stat().st_size != patch_binding["bytes"] or sha256(patch_executable) != patch_binding["sha256"]:
        raise ContextAssemblyError(f"bound patch executable hash differs: {patch_executable}")

    first_temp = None
    second_temp = None
    receipt_path = context_receipt_path(root, purpose)
    try:
        first_temp, first_context, first = _assemble_tree(repo_root=repo_root, external=external, patch_executable=patch_executable, width=width, r5=r5, purpose=purpose)
        second_temp, _second_context, second = _assemble_tree(repo_root=repo_root, external=external, patch_executable=patch_executable, width=width, r5=r5, purpose=f"{purpose}-replay")
        first_identity = _source_identities(first["sourceHashes"])
        second_identity = _source_identities(second["sourceHashes"])
        same_source_hashes = first_identity == second_identity
        if not same_source_hashes:
            differences = {
                name: {"first": first_identity.get(name), "second": second_identity.get(name)}
                for name in sorted(set(first_identity) | set(second_identity))
                if first_identity.get(name) != second_identity.get(name)
            }
            atomic_json(receipt_path, {"schema": ASSEMBLY_SCHEMA, "createdAt": now(), "experimentId": "strength-first-step12-independent-replication-20260916-v1", "status": "failed", "purpose": purpose, "firstAssembly": first, "independentReplay": second, "sourceHashDifferences": differences, "error": "independent context assembly produced different source hashes", "flags": {"confirmationAuthorized": False, "deploymentAuthorized": False, "outcomesOpened": False, "handcraftedRemainsProduction": True}})
            raise ContextAssemblyError(f"independent context assembly produced different source hashes: {sorted(differences)}")
        receipt = {
            "schema": ASSEMBLY_SCHEMA,
            "createdAt": now(),
            "experimentId": "strength-first-step12-independent-replication-20260916-v1",
            "status": "assembled-before-cmake",
            "purpose": purpose,
            "externalCheckout": {"path": str(external), "revision": first["revision"]},
            "patchExecutable": first["patchExecutable"],
            "patchVersion": first["patchVersion"],
            "firstAssembly": first,
            "independentReplay": second,
            "sourceIdentities": first_identity,
            "sameSourceHashes": same_source_hashes,
            "cmake": {"executable": shutil.which(cmake) or cmake, "pending": True},
            "flags": {"confirmationAuthorized": False, "deploymentAuthorized": False, "outcomesOpened": False, "handcraftedRemainsProduction": True},
        }
        atomic_json(receipt_path, receipt)
        cmake_records = []
        for argv in (
            [cmake, "-S", ".", "-B", "build", "-DCMAKE_POLICY_VERSION_MINIMUM=3.5"],
            [cmake, "--build", "build"],
            [cmake, "--install", "build", "--prefix", "."],
        ):
            record = _command(argv, first_context)
            _require_success(record, " ".join(argv))
            cmake_records.append(record)
        loader_outputs = [artifact(path) for path in first_context.rglob("*") if path.is_file() and (path.suffix in {".so", ".dylib"} or "nnue_dataset" in path.name)]
        receipt["status"] = "built"
        receipt["cmake"] = {"executable": shutil.which(cmake) or cmake, "commands": cmake_records, "loaderOutputs": loader_outputs}
        atomic_json(receipt_path, receipt)
        second_temp.cleanup()
        second_temp = None
        return first_temp, first_context
    except Exception as exc:
        failure = {"schema": ASSEMBLY_SCHEMA, "createdAt": now(), "experimentId": "strength-first-step12-independent-replication-20260916-v1", "status": "failed", "purpose": purpose, "error": str(exc), "flags": {"confirmationAuthorized": False, "deploymentAuthorized": False, "outcomesOpened": False, "handcraftedRemainsProduction": True}}
        atomic_json(receipt_path, failure)
        if second_temp is not None:
            second_temp.cleanup()
        if first_temp is not None:
            first_temp.cleanup()
        raise
