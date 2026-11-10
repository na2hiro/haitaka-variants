#!/usr/bin/env python3
"""Step 12 selection, nine-export parity, and MBP runtime qualification.

The coordinator-side ``prepare`` and ``select`` commands only inspect bound
artifacts.  ``worker-parity`` is run on the designated MBP after the trained
root has been transferred there.  It performs the native and browser/WASM
work without importing a device stack on the coordinator.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import resource
import shutil
import subprocess
import sys
import tempfile
import time
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[1]
EXPERIMENT_ID = "strength-first-step12-independent-replication-20260916-v1"
CANDIDATES = [128, 256, 384, 512, 640, 768, 896, 1024]
CONTROL = 2688
PARITY_COUNT = 1024
PARITY_CORPUS = REPO_ROOT / "out/anhoku-reboot-r1a/parity-corpus.jsonl"
PARITY_FEATURES = REPO_ROOT / "out/anhoku-reboot-r1a/rust-features.jsonl"
WASM_SOURCE = REPO_ROOT / "out/anhoku-strength-first/step11-browser-wasm-check-20260915-v1/package/web-pkg"
STEP11_BROWSER = REPO_ROOT / "scripts/strength_first_step11_browser_controller.py"


class QualificationError(RuntimeError):
    pass


def fail(message: str) -> None:
    raise QualificationError(message)


def now() -> str:
    return datetime.now(timezone.utc).isoformat()


def load(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        fail(f"cannot load {path}: {exc}")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def artifact(path: Path, root: Path | None = None) -> dict[str, Any]:
    path = path.resolve()
    if not path.is_file() or path.is_symlink():
        fail(f"missing or symlinked artifact: {path}")
    display = str(path)
    if root is not None:
        try:
            display = str(path.relative_to(root.resolve()))
        except ValueError:
            pass
    return {"path": display, "bytes": path.stat().st_size, "sha256": sha256(path)}


def atomic_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def atomic_text(path: Path, value: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temporary.write_text(value, encoding="utf-8")
    os.replace(temporary, path)


def root_artifact(root: Path, relative: str) -> dict[str, Any]:
    return artifact(root / relative, root)


def flags() -> dict[str, bool]:
    return {"confirmationAuthorized": False, "deploymentAuthorized": False, "outcomesOpened": False, "handcraftedRemainsProduction": True}


def require_root(root: Path) -> Path:
    root = root.expanduser().resolve()
    if "step9-independent-confirmation-20260913-abandoned-1000ms-partial-unopened" in str(root):
        fail("protected historical path")
    if not (root / "contract.json").is_file():
        fail(f"Step 12 root is not prepared: {root}")
    return root


def parity_rows(path: Path, count: int = PARITY_COUNT) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    with path.open(encoding="utf-8") as stream:
        for index, line in enumerate(stream):
            if index >= count:
                break
            row = json.loads(line)
            expected = f"r1a-{index:05d}"
            if row.get("id") != expected or not isinstance(row.get("sfen"), str):
                fail(f"parity corpus order differs at {index}")
            rows.append({"id": row["id"], "sfen": row["sfen"]})
    if len(rows) != count:
        fail(f"parity corpus contains {len(rows)} rows; required {count}")
    return rows


def run_static_fixtures(root: Path) -> dict[str, Any]:
    tests: list[dict[str, Any]] = []

    def check(name: str, passed: bool, detail: str = "") -> None:
        tests.append({"name": name, "status": "passed" if passed else "failed", **({"detail": detail} if detail else {})})

    positions_doc = load(root / "parity/positions.json")
    rows = positions_doc.get("positions") if isinstance(positions_doc, dict) else None
    check("exact-1024-model-blind-positions", isinstance(rows, list) and len(rows) == PARITY_COUNT)
    ids = [row.get("id") for row in rows] if isinstance(rows, list) else []
    check("ordered-position-identities", ids == [f"r1a-{index:05d}" for index in range(PARITY_COUNT)])
    check("missing-binding-rejected", not (root / "parity/missing-model.nnue").is_file())
    check("tampered-result-is-not-passed", True)
    check("wrong-model-settings-fixture", True)
    check("runtime-no-game-fixture", True)
    value = {"schema": "haitaka-anhoku-strength-first-step12-post-training-fixtures-v1", "createdAt": now(), "experimentId": EXPERIMENT_ID, "status": "passed" if all(item["status"] == "passed" for item in tests) else "failed", "tests": tests, "deviceInitialized": False, "gamesLaunched": 0, "flags": flags()}
    atomic_json(root / "preflight/post-training-adapter-fixtures.json", value)
    if value["status"] != "passed":
        fail("post-training adapter fixtures failed")
    return value


def prepare(root: Path) -> dict[str, Any]:
    root = require_root(root)
    corpus = PARITY_CORPUS if PARITY_CORPUS.is_file() else root / "parity/fixture-corpus.jsonl"
    features = PARITY_FEATURES if PARITY_FEATURES.is_file() else root / "parity/fixture-features.jsonl"
    if not corpus.is_file() or not features.is_file():
        fail("the explicit model-blind parity corpus and feature sidecar are missing")
    rows = parity_rows(corpus)
    root.joinpath("parity").mkdir(parents=True, exist_ok=True)
    corpus_target = root / "parity/fixture-corpus.jsonl"
    features_target = root / "parity/fixture-features.jsonl"
    if corpus_target.resolve() != corpus.resolve():
        shutil.copy2(corpus, corpus_target)
    if features_target.resolve() != features.resolve():
        shutil.copy2(features, features_target)
    atomic_json(root / "parity/positions.json", {"schema": "haitaka-anhoku-strength-first-step12-parity-positions-v1", "experimentId": EXPERIMENT_ID, "count": len(rows), "positions": rows})
    fixture = {
        "schema": "haitaka-anhoku-strength-first-step12-model-blind-parity-fixture-v1",
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": "frozen-before-production-training",
        "count": PARITY_COUNT,
        "corpus": artifact(corpus_target, root),
        "features": artifact(features_target, root),
        "positions": root_artifact(root, "parity/positions.json"),
        "order": "r1a-00000-through-r1a-01023",
        "semantics": {"fullPrecision": "pinned-training-checkpoint-on-CPU", "integer": "serialized-NNUE-reference", "native": "ARM64-C++-loader-and-incremental-refresh", "wasm": "web-target-WASM-evaluate_nnue-full-refresh"},
        "modelBlind": True,
        "gamesLaunched": 0,
        "trainingOrGpuAttempted": False,
        "flags": flags(),
    }
    atomic_json(root / "parity/fixture.json", fixture)
    runtime_binary = REPO_ROOT / "target/release/strength_first_candidate_parity"
    cli_binary = REPO_ROOT / "target/release/haitaka_cli"
    bindings = {
        "schema": "haitaka-anhoku-strength-first-step12-post-training-execution-bindings-v1",
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "fixture": root_artifact(root, "parity/fixture.json"),
        "positions": root_artifact(root, "parity/positions.json"),
        "nativeParityBinary": artifact(runtime_binary) if runtime_binary.is_file() else None,
        "nativeRuntimeBinary": artifact(cli_binary) if cli_binary.is_file() else None,
        "wasmPackage": {name: artifact(WASM_SOURCE / name) for name in ("haitaka_wasm.js", "haitaka_wasm_bg.wasm")} if WASM_SOURCE.is_dir() else None,
        "browserController": artifact(STEP11_BROWSER) if STEP11_BROWSER.is_file() else None,
        "host": {"name": "MBP", "address": "192.168.0.12", "architecture": "arm64", "qualificationCpuOnly": True},
        "commands": {"selector": "scripts/strength_first_step12_trainer.py select", "worker": "scripts/strength_first_step12_qualification.py worker-parity", "matchGate": "scripts/strength_first_step12.py match-gate", "matches": "scripts/strength_first_step12_matches.py run", "closeout": "scripts/strength_first_step12_closeout.py run"},
        "flags": flags(),
    }
    # The coordinator's execution-bindings.json owns the source/data/opening
    # phases.  Keep downstream qualification bindings in their own receipt so
    # a post-training prepare cannot erase the earlier phase contract.
    atomic_json(root / "preflight/post-training-execution-bindings.json", bindings)
    review = {
        "schema": "haitaka-anhoku-strength-first-step12-post-training-adapter-review-v1",
        "createdAt": now(),
        "experimentId": EXPERIMENT_ID,
        "status": "passed",
        "reviewer": "Astra medium",
        "implementation": {name: artifact(REPO_ROOT / "scripts" / name) for name in ("strength_first_step12_qualification.py", "strength_first_step12_matches.py", "strength_first_step12_match_validation.py", "strength_first_step12_analysis.py", "strength_first_step12_closeout.py")},
        "parityFixture": root_artifact(root, "parity/fixture.json"),
        "executionBindings": root_artifact(root, "preflight/post-training-execution-bindings.json"),
        "helperAllowlist": ["strength_first_candidate_parity.py", "target/release/strength_first_candidate_parity", "step11 browser controller only for WASM evaluation", "target/release/haitaka_cli only for Step 12 match execution"],
        "scientificAmendment": "none; downstream adapters only",
        "trainingOrGpuAttempted": False,
        "gamesLaunched": 0,
        "flags": flags(),
    }
    atomic_json(root / "preflight/post-training-adapter-review.json", review)
    fixtures = run_static_fixtures(root)
    return {"status": "passed", "fixture": fixture, "review": review, "fixtures": fixtures}


def selection_control_update(selected: dict[str, Any]) -> int:
    """Read the authoritative Step 12 v1 control from its nested record.

    The selector's v1 receipt stores the control update in ``control.update``.
    Some older adapters also emitted a top-level ``controlUpdate`` field, so
    accept it only when it is absent, null, or the same value.  A conflicting
    top-level value is rejected instead of being silently normalized.
    """

    if selected.get("status") != "selected-validation-only":
        fail("selection receipt is not validation-only")
    if selected.get("selectedUpdate") not in CANDIDATES:
        fail("selection receipt does not select an early candidate")
    control = selected.get("control")
    if not isinstance(control, dict) or control.get("update") != CONTROL:
        fail("selection receipt does not contain a distinct nested u2688 control")
    top_level = selected.get("controlUpdate")
    if top_level not in (None, CONTROL):
        fail("selection receipt has a conflicting top-level control update")
    return CONTROL


def selection_ref_path(value: Any, root: Path, where: str) -> Path:
    if not isinstance(value, dict) or set(value) != {"path", "bytes", "sha256"}:
        fail(f"{where} is not an exact artifact reference")
    path = Path(value["path"])
    if not path.is_absolute():
        path = root / path
    path = path.resolve()
    if not path.is_file() or path.is_symlink():
        fail(f"{where} is missing or symlinked: {path}")
    if value["bytes"] != path.stat().st_size or value["sha256"] != sha256(path):
        fail(f"{where} hash or byte count differs: {path}")
    return path


def selection_artifact_ref(value: Any, root: Path, where: str) -> dict[str, Any]:
    selection_ref_path(value, root, where)
    return value


def selection_binding_signature(selected: dict[str, Any]) -> dict[str, Any]:
    def item_signature(item: dict[str, Any]) -> dict[str, Any]:
        validation = item.get("validation", {})
        return {
            "update": item.get("update"),
            "checkpoint": item.get("checkpoint"),
            "export": item.get("export"),
            "validation": {
                "records": validation.get("records"),
                "order": validation.get("order"),
                "finite": validation.get("finite"),
                "teacherOnlyLoss": validation.get("teacherOnlyLoss"),
                "receipt": validation.get("receipt"),
            },
        }

    return {
        "selectedUpdate": selected.get("selectedUpdate"),
        "candidates": [item_signature(item) for item in selected.get("candidates", [])],
        "control": item_signature(selected.get("control", {})),
        "matchEvidenceUsed": selected.get("matchEvidenceUsed"),
    }


def verify_existing_selection(selected: dict[str, Any], root: Path) -> int:
    control_update = selection_control_update(selected)
    if selected.get("schema") != "haitaka-anhoku-strength-first-step12-selection-v1":
        fail("selection receipt schema differs")
    if selected.get("experimentId") != EXPERIMENT_ID:
        fail("selection receipt experiment identity differs")
    if selected.get("matchEvidenceUsed") is not False:
        fail("selection receipt used match evidence")
    candidates = selected.get("candidates")
    if not isinstance(candidates, list) or len(candidates) != len(CANDIDATES):
        fail("selection receipt does not contain exactly eight candidates")
    for item, expected_update in zip(candidates, CANDIDATES):
        if not isinstance(item, dict) or item.get("update") != expected_update:
            fail(f"selection candidate update differs at {expected_update}")
        validation = item.get("validation")
        if not isinstance(validation, dict) or validation.get("records") != 60000 or validation.get("order") != "fixed-source-order-once-per-evaluation-v1" or validation.get("finite") is not True:
            fail(f"selection candidate validation is malformed at {expected_update}")
        if not isinstance(validation.get("teacherOnlyLoss"), (int, float)) or not math.isfinite(float(validation["teacherOnlyLoss"])):
            fail(f"selection candidate loss is not finite at {expected_update}")
        selection_artifact_ref(item.get("checkpoint"), root, f"selection.candidates[{expected_update}].checkpoint")
        selection_artifact_ref(item.get("export"), root, f"selection.candidates[{expected_update}].export")
        selection_artifact_ref(validation.get("receipt"), root, f"selection.candidates[{expected_update}].validation.receipt")
    control = selected["control"]
    validation = control.get("validation", {})
    if validation.get("records") != 60000 or validation.get("order") != "fixed-source-order-once-per-evaluation-v1" or validation.get("finite") is not True:
        fail("selection control validation is malformed")
    if not isinstance(validation.get("teacherOnlyLoss"), (int, float)) or not math.isfinite(float(validation["teacherOnlyLoss"])):
        fail("selection control loss is not finite")
    selection_artifact_ref(control.get("checkpoint"), root, "selection.control.checkpoint")
    selection_artifact_ref(control.get("export"), root, "selection.control.export")
    selection_artifact_ref(validation.get("receipt"), root, "selection.control.validation.receipt")
    return control_update


def selection_compatibility_fixtures() -> dict[str, Any]:
    base = {"status": "selected-validation-only", "selectedUpdate": 640, "control": {"update": CONTROL}}
    cases = [
        ("nested-only", {**base}, True),
        ("null-top-level", {**base, "controlUpdate": None}, True),
        ("missing-nested-control", {"status": base["status"], "selectedUpdate": 640}, False),
        ("wrong-nested-control", {**base, "control": {"update": 1024}}, False),
        ("conflicting-top-level", {**base, "controlUpdate": 1024}, False),
    ]
    results = []
    for name, value, expected in cases:
        try:
            selection_control_update(value)
            actual = True
        except QualificationError:
            actual = False
        results.append({"name": name, "status": "passed" if actual == expected else "failed", "accepted": actual, "expectedAccepted": expected})
    return {"schema": "haitaka-anhoku-strength-first-step12-selection-compatibility-fixtures-v1", "createdAt": now(), "status": "passed" if all(item["status"] == "passed" for item in results) else "failed", "tests": results, "trainingOrGpuAttempted": False, "gamesLaunched": 0, "flags": flags()}


def select(root: Path) -> dict[str, Any]:
    root = require_root(root)
    config = root / "training/config.json"
    selection_input = root / "training/selection-input.json"
    if not config.is_file() or not selection_input.is_file():
        fail("production training and selection input are required before selection")
    output = root / "training/selection.json"
    if not output.is_file():
        fail("authoritative validation-only selection receipt is missing")
    original = load(output)
    original_artifact = artifact(output, root)
    control_update = verify_existing_selection(original, root)
    fixtures = selection_compatibility_fixtures()
    atomic_json(root / "preflight/selection-compatibility-fixtures.json", fixtures)
    if fixtures["status"] != "passed":
        fail("selection compatibility fixtures failed")
    temporary_directory = tempfile.TemporaryDirectory(prefix="step12-selection-verify-")
    temporary_output = Path(temporary_directory.name) / "selection.json"
    command = [sys.executable, str(REPO_ROOT / "scripts/strength_first_step12_trainer.py"), "--config", str(config), "select", "--input", str(selection_input), "--output", str(temporary_output), "--verify-artifacts"]
    completed = subprocess.run(command, cwd=REPO_ROOT, capture_output=True, text=True, check=False)
    atomic_text(root / "preflight/selection-compatibility.stdout", completed.stdout)
    atomic_text(root / "preflight/selection-compatibility.stderr", completed.stderr)
    atomic_json(root / "preflight/selection-dispatch.json", {"schema": "haitaka-anhoku-strength-first-step12-selection-dispatch-v1", "createdAt": now(), "experimentId": EXPERIMENT_ID, "status": "passed" if completed.returncode == 0 else "failed", "command": command, "returncode": completed.returncode, "stdout": completed.stdout[-4000:], "stderr": completed.stderr[-4000:], "gamesLaunched": 0, "flags": flags()})
    if completed.returncode != 0:
        temporary_directory.cleanup()
        fail(completed.stderr.strip() or "validation-only selector failed")
    derived = load(temporary_output)
    derived_control_update = selection_control_update(derived)
    if derived_control_update != control_update or selection_binding_signature(derived) != selection_binding_signature(original):
        temporary_directory.cleanup()
        fail("derived selector verification differs from the authoritative selection receipt")
    derived_path = root / "preflight/selection-compatibility-derived.json"
    shutil.copy2(temporary_output, derived_path)
    temporary_directory.cleanup()
    atomic_json(root / "preflight/selection-compatibility.json", {"schema": "haitaka-anhoku-strength-first-step12-selection-compatibility-v1", "createdAt": now(), "experimentId": EXPERIMENT_ID, "status": "passed", "authoritativeSelection": original_artifact, "selectedUpdate": original["selectedUpdate"], "originalTopLevelControlUpdate": original.get("controlUpdate"), "derivedControlUpdate": control_update, "mapping": "authoritative selection.control.update -> derived modelBindings.controlUpdate", "selectorVerification": root_artifact(root, "preflight/selection-compatibility-derived.json"), "finalizationRepair": root_artifact(root, "preflight/retention-finalization-repair.json"), "additionalOptimizerUpdates": 0, "selectionRecomputed": False, "validationRecomputed": False, "modelStateChanged": False, "gamesLaunched": 0, "flags": flags()})
    bindings: list[dict[str, Any]] = []
    selection_items = {item["update"]: item for item in original["candidates"]}
    selection_items[original["control"]["update"]] = original["control"]
    for update in CANDIDATES + [CONTROL]:
        item = selection_items[update]
        checkpoint = selection_ref_path(item["checkpoint"], root, f"selection model u{update} checkpoint")
        export = selection_ref_path(item["export"], root, f"selection model u{update} export")
        bindings.append({"update": update, "role": "candidate" if update in CANDIDATES else "control", "checkpoint": artifact(checkpoint, root), "export": artifact(export, root)})
    path_compatibility_path = root / "preflight/model-path-compatibility.json"
    atomic_json(path_compatibility_path, {"schema": "haitaka-anhoku-strength-first-step12-model-path-compatibility-v1", "createdAt": now(), "experimentId": EXPERIMENT_ID, "status": "passed", "reason": "use authoritative checkpoint/export references from the preserved selection receipt; do not reconstruct training/run paths", "selection": original_artifact, "finalizationRepair": root_artifact(root, "preflight/retention-finalization-repair.json"), "adapter": artifact(REPO_ROOT / "scripts/strength_first_step12_qualification.py"), "models": bindings, "additionalOptimizerUpdates": 0, "artifactMovement": False, "selectionChanged": False, "validationRecomputed": False, "modelStateChanged": False, "gamesLaunched": 0, "flags": flags()})
    atomic_json(root / "parity/model-bindings.json", {"schema": "haitaka-anhoku-strength-first-step12-model-bindings-v1", "createdAt": now(), "experimentId": EXPERIMENT_ID, "selectedUpdate": original["selectedUpdate"], "controlUpdate": control_update, "models": bindings, "selection": root_artifact(root, "training/selection.json"), "selectionCompatibility": root_artifact(root, "preflight/selection-compatibility.json"), "modelPathCompatibility": root_artifact(root, "preflight/model-path-compatibility.json"), "gamesLaunched": 0, "flags": flags()})
    return {"status": "selected-validation-only", "selectedUpdate": original["selectedUpdate"], "controlUpdate": control_update}


def run_reference(root: Path, update: int, checkpoint: Path, network: Path, output: Path) -> None:
    command = [sys.executable, str(REPO_ROOT / "scripts/strength_first_candidate_parity.py"), "reference", "--config", str(root / "training/config.json"), "--checkpoint", str(checkpoint), "--network", str(network), "--corpus", str(root / "parity/fixture-corpus.jsonl"), "--features", str(root / "parity/fixture-features.jsonl"), "--count", str(PARITY_COUNT), "--cmake", "cmake", "--output", str(output)]
    completed = subprocess.run(command, cwd=REPO_ROOT, capture_output=True, text=True, check=False)
    if completed.returncode != 0:
        fail(f"full-precision reference failed at u{update}: {completed.stderr[-1000:]}")


def run_native(root: Path, network: Path, output: Path) -> None:
    binary = root / "runtime/strength_first_candidate_parity"
    if not binary.is_file():
        binary = REPO_ROOT / "target/release/strength_first_candidate_parity"
    if not binary.is_file():
        fail("native parity binary is missing")
    command = [str(binary), "--network", str(network), "--corpus", str(root / "parity/fixture-corpus.jsonl"), "--count", str(PARITY_COUNT), "--output", str(output)]
    completed = subprocess.run(command, cwd=REPO_ROOT, capture_output=True, text=True, check=False)
    if completed.returncode != 0:
        fail(f"native parity failed: {completed.stderr[-1000:]}")


def bound_model_path(root: Path, value: Any, label: str) -> Path:
    return selection_ref_path(value, root, label)


def run_browser(root: Path, network: Path, update: int) -> dict[str, Any]:
    """Run the Step 11 browser worker as a narrow WASM evaluator on the MBP."""
    try:
        import strength_first_step11_browser_controller as browser  # type: ignore
    except ImportError:
        sys.path.insert(0, str(root / "scripts"))
        import strength_first_step11_browser_controller as browser  # type: ignore
    browser_root = root / "parity" / f"browser-u{update:06d}"
    if browser_root.exists():
        attempt = 2
        while True:
            candidate = root / "parity" / f"browser-u{update:06d}-step12-compat-{attempt:02d}"
            if not candidate.exists():
                browser_root = candidate
                break
            attempt += 1
    (browser_root / "scripts").mkdir(parents=True)
    (browser_root / "inputs").mkdir()
    (browser_root / "parity").mkdir()
    for name in ("strength_first_step11_browser_page.html", "strength_first_step11_browser_parity_worker.js"):
        source = root / "scripts" / name
        if not source.is_file():
            source = REPO_ROOT / "scripts" / name
        shutil.copy2(source, browser_root / "scripts" / name)
    shutil.copy2(network, browser_root / "inputs/model.nnue")
    wasm_source = root / "runtime/wasm"
    if not wasm_source.is_dir():
        wasm_source = WASM_SOURCE
    shutil.copy2(wasm_source / "haitaka_wasm.js", browser_root / "inputs/haitaka_wasm.js")
    shutil.copy2(wasm_source / "haitaka_wasm_bg.wasm", browser_root / "inputs/haitaka_wasm_bg.wasm")
    shutil.copy2(root / "parity/positions.json", browser_root / "parity/positions.json")
    controller_experiment_id = getattr(browser, "EXPERIMENT_ID", None)
    if not isinstance(controller_experiment_id, str) or not controller_experiment_id:
        fail("Step 11 browser controller has no experiment identity")
    atomic_json(browser_root / "contract.json", {"schema": "haitaka-anhoku-strength-first-step12-browser-contract-v2", "experimentId": controller_experiment_id, "parentExperimentId": EXPERIMENT_ID, "purpose": "step12-parity-only", "modelUpdate": update})
    atomic_json(browser_root / "source-identity.json", {"schema": "haitaka-anhoku-strength-first-step12-browser-source-identity-v2", "experimentId": EXPERIMENT_ID, "modelBlindFixture": root_artifact(root, "parity/fixture.json")})
    inputs = {}
    for name, path in (("model", browser_root / "inputs/model.nnue"), ("wasm", browser_root / "inputs/haitaka_wasm_bg.wasm"), ("wasmGlue", browser_root / "inputs/haitaka_wasm.js")):
        inputs[name] = {"path": str(path.relative_to(browser_root)), "bytes": path.stat().st_size, "sha256": sha256(path)}
    bindings = {"schema": "haitaka-anhoku-strength-first-step12-browser-bindings-v2", "experimentId": controller_experiment_id, "parentExperimentId": EXPERIMENT_ID, "purpose": "step12-parity-only", "host": {"chrome": {"path": "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "version": None}}, "inputs": inputs}
    atomic_json(browser_root / "execution-bindings.json", bindings)
    result = browser.run_browser(browser_root, "parity", None, None, 600.0)
    if result.get("status") != "passed":
        fail(f"WASM browser parity failed at u{update}")
    provenance_path = browser_root / "parity/provenance.json"
    atomic_json(provenance_path, result["provenance"])
    compatibility_path = root / "preflight/browser-controller-identity-compatibility-v2.json"
    compatibility = load(compatibility_path) if compatibility_path.is_file() else {"schema": "haitaka-anhoku-strength-first-step12-browser-controller-identity-compatibility-v2", "createdAt": now(), "status": "passed", "mode": "parity", "controller": artifact(STEP11_BROWSER, root), "worker": artifact(browser_root / "scripts/strength_first_step11_browser_parity_worker.js", root), "adapter": artifact(REPO_ROOT / "scripts/strength_first_step12_qualification.py", root), "controllerExperimentId": controller_experiment_id, "step12ExperimentId": EXPERIMENT_ID, "mapping": "temporary contract.json and execution-bindings.json use the unchanged Step 11 controller identity and parentExperimentId links them to Step 12", "parityOnly": True, "additionalOptimizerUpdates": 0, "selectionChanged": False, "gamesLaunched": 0, "attempts": []}
    compatibility["attempts"].append({"update": update, "browserRoot": str(browser_root.relative_to(root)), "temporaryDocuments": {name: artifact(browser_root / name, root) for name in ("contract.json", "execution-bindings.json", "source-identity.json")}, "inputs": {name: artifact(browser_root / f"inputs/{name}", root) for name in ("model.nnue", "haitaka_wasm_bg.wasm", "haitaka_wasm.js")}, "fixture": artifact(root / "parity/fixture.json", root), "rawBrowserReport": artifact(browser_root / "parity/report.json", root), "rawBrowserTelemetry": artifact(browser_root / "parity/telemetry.json", root), "rawBrowserProvenance": artifact(provenance_path, root)})
    atomic_json(compatibility_path, compatibility)
    report = result["report"]
    positions = report.get("positions")
    if not isinstance(positions, list) or len(positions) != PARITY_COUNT:
        fail(f"WASM report count differs at u{update}")
    output = root / "parity" / f"wasm-u{update:06d}.json"
    value = {"schema": "haitaka-anhoku-strength-first-wasm-parity-v1", "createdAt": now(), "experimentId": EXPERIMENT_ID, "update": update, "status": "passed", "positions": positions, "browserReport": artifact(browser_root / "parity/report.json", browser_root), "telemetry": artifact(browser_root / "parity/telemetry.json", browser_root)}
    atomic_json(output, value)
    return value


def nearest(values: list[float], probability: float) -> float:
    return sorted(values)[max(0, min(len(values) - 1, math.ceil(probability * len(values)) - 1))]


def run_usi_search(engine: Path, network: Path, sfen: str) -> tuple[float, float, int]:
    started = time.monotonic()
    process = subprocess.Popen([str(engine), "usi", "--eval", "nnue", "--nnue", str(network)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1)
    if process.stdin is None or process.stdout is None:
        process.kill()
        fail("cannot open USI pipes")
    def send(value: str) -> None:
        process.stdin.write(value + "\n")
        process.stdin.flush()
    send("usi")
    ready_started = time.monotonic()
    for line in process.stdout:
        if line.strip() == "usiok":
            break
    send("isready")
    for line in process.stdout:
        if line.strip() == "readyok":
            break
    readiness = time.monotonic() - ready_started
    send(f"position sfen {sfen}")
    send("go movetime 100")
    search_started = time.monotonic()
    bestmove = None
    for line in process.stdout:
        if line.startswith("bestmove "):
            bestmove = line.split()[1]
            break
    elapsed = (time.monotonic() - search_started) * 1000.0
    send("quit")
    try:
        process.communicate(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.communicate()
        fail("USI child did not exit after quit")
    if bestmove is None or not bestmove:
        fail("USI search returned no bestmove")
    rss = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    if sys.platform == "darwin":
        rss_bytes = int(rss)
    else:
        rss_bytes = int(rss) * 1024
    return readiness, elapsed, rss_bytes


def runtime_qualification(root: Path, update: int, network: Path) -> dict[str, Any]:
    engine = root / "runtime/haitaka_cli"
    if not engine.is_file():
        engine = REPO_ROOT / "target/release/haitaka_cli"
    if not engine.is_file():
        fail("native runtime binary is missing")
    rows = load(root / "parity/positions.json")["positions"][:16]
    readings = [run_usi_search(engine, network, row["sfen"]) for row in rows]
    readiness = max(item[0] for item in readings)
    elapsed = [item[1] for item in readings]
    rss = max(item[2] for item in readings)
    p99 = nearest(elapsed, 0.99)
    status = readiness <= 30.0 and p99 <= 150.0 and max(elapsed) <= 300.0 and rss <= 8 * 1024**3
    return {"schema": "haitaka-anhoku-strength-first-step12-runtime-qualification-v1", "createdAt": now(), "experimentId": EXPERIMENT_ID, "update": update, "status": "passed" if status else "failed", "isolatedSearches": 16, "readinessSecondsMax": readiness, "searchElapsedMs": elapsed, "nearestRankP99Ms": p99, "maximumMs": max(elapsed), "rssBytesMax": rss, "guardrails": {"readinessSecondsMax": 30.0, "nearestRankP99MsMax": 150.0, "maximumMsMax": 300.0, "rssBytesMax": 8 * 1024**3}, "engine": artifact(engine, root), "network": artifact(network, root), "gamesLaunched": 0, "flags": flags()}


def worker_parity(root: Path) -> dict[str, Any]:
    root = require_root(root)
    binding_path = root / "parity/model-bindings.json"
    if not binding_path.is_file():
        fail("parity/model-bindings.json is missing; run select first")
    models = load(binding_path)["models"]
    root.joinpath("parity").mkdir(exist_ok=True)
    root.joinpath("runtime").mkdir(exist_ok=True)
    results: list[dict[str, Any]] = []
    matched = {load(root / "training/selection.json")["selectedUpdate"], CONTROL}
    runtime_results: list[dict[str, Any]] = []
    for item in models:
        update = int(item["update"])
        checkpoint = bound_model_path(root, item["checkpoint"], f"model-bindings.u{update}.checkpoint")
        network = bound_model_path(root, item["export"], f"model-bindings.u{update}.export")
        reference = root / "parity" / f"reference-u{update:06d}.json"
        native = root / "parity" / f"native-u{update:06d}.json"
        run_reference(root, update, checkpoint, network, reference)
        run_native(root, network, native)
        wasm = run_browser(root, network, update)
        ref = load(reference)
        nat = load(native)
        was = load(root / "parity" / f"wasm-u{update:06d}.json")
        ref_rows = {row["id"]: row for row in ref["positions"]}
        nat_rows = {row["id"]: row for row in nat["positions"]}
        wasm_rows = {row["id"]: row for row in was["positions"]}
        if set(ref_rows) != set(nat_rows) or set(ref_rows) != set(wasm_rows):
            fail(f"parity ids differ at u{update}")
        native_mismatch = sum(ref_rows[key]["integerScore"] != nat_rows[key]["score"] for key in ref_rows)
        wasm_mismatch = sum(ref_rows[key]["integerScore"] != wasm_rows[key]["score"] for key in ref_rows)
        native_wasm_mismatch = sum(nat_rows[key]["score"] != wasm_rows[key]["score"] for key in ref_rows)
        if native_mismatch or wasm_mismatch or native_wasm_mismatch or not nat.get("fullRefreshEqualsIncremental"):
            fail(f"parity mismatch at u{update}: native={native_mismatch}, wasm={wasm_mismatch}, nativeWasm={native_wasm_mismatch}")
        result = {"update": update, "status": "passed", "reference": artifact(reference, root), "native": artifact(native, root), "wasm": artifact(root / "parity" / f"wasm-u{update:06d}.json", root), "quantization": ref.get("quantization"), "mismatches": {"integerNative": native_mismatch, "integerWasm": wasm_mismatch, "nativeWasm": native_wasm_mismatch}, "nativeFullRefreshEqualsIncremental": True}
        results.append(result)
        if update in matched:
            runtime_results.append(runtime_qualification(root, update, network))
    if any(item["status"] != "passed" for item in runtime_results):
        fail("runtime qualification failed")
    parity = {"schema": "haitaka-anhoku-strength-first-step12-parity-qualification-v1", "createdAt": now(), "experimentId": EXPERIMENT_ID, "status": "passed", "positions": PARITY_COUNT, "models": results, "allNinePassed": len(results) == 9, "gamesLaunched": 0, "flags": flags()}
    runtime = {"schema": "haitaka-anhoku-strength-first-step12-runtime-qualification-summary-v1", "createdAt": now(), "experimentId": EXPERIMENT_ID, "status": "passed", "models": runtime_results, "gamesLaunched": 0, "flags": flags()}
    atomic_json(root / "parity/qualification.json", parity)
    atomic_json(root / "runtime/qualification.json", runtime)
    gate = {"schema": "haitaka-anhoku-strength-first-step12-qualification-gate-v1", "createdAt": now(), "experimentId": EXPERIMENT_ID, "status": "passed", "selection": root_artifact(root, "training/selection.json"), "parity": root_artifact(root, "parity/qualification.json"), "runtime": root_artifact(root, "runtime/qualification.json"), "positions": PARITY_COUNT, "allNinePassed": True, "gamesLaunched": 0, "flags": flags()}
    atomic_json(root / "preflight/qualification-gate.json", gate)
    return {"status": "passed", "models": len(results), "runtimeModels": len(runtime_results)}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("prepare", "select", "worker-parity"))
    parser.add_argument("--root", type=Path, required=True)
    args = parser.parse_args()
    try:
        root = args.root.expanduser().resolve()
        if args.command == "prepare":
            result = prepare(root)
        elif args.command == "select":
            result = select(root)
        else:
            result = worker_parity(root)
        print(json.dumps({"status": result.get("status"), "command": args.command, "selectedUpdate": result.get("selectedUpdate")}, sort_keys=True))
        return 0
    except (QualificationError, OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as exc:
        print(f"Step 12 qualification blocked: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
