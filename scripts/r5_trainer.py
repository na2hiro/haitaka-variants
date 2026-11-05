#!/usr/bin/env python3
"""Dedicated R5 trainer for the immutable replacement-R4 pilot.

The legacy ``haitaka_learn train`` command intentionally is not used here: it
accepts the older game-count manifest and its trainer objective is the retired
sigmoid loss.  This runner builds a disposable pinned trainer context, uses the
finite C++ stream directly, applies the frozen robust Huber objective, and
records exact physical exposure at every checkpoint.

The command requires ``--device cuda``.  It is therefore safe to invoke during
CPU preparation: it fails before a training batch when CUDA is unavailable.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import random
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path
from typing import Any, Iterable


# Must be set before torch is imported.  The R2 cross-hardware gate found that
# cuBLAS deterministic mode otherwise depends on process startup order.
os.environ.setdefault("CUBLAS_WORKSPACE_CONFIG", ":4096:8")

import numpy as np


ROOT = Path(__file__).resolve().parents[1]
PINNED_TRAINER = "61666d9e3653e4df9881b14c23f8fdcc4bf7779b"
FEATURE_SET = "HalfKAv2^+DonorSingleEff"
BATCH_SIZE = 16384
ENTRY_BYTES = 72
SCORE_CLIP = 12000.0
HUBER_DELTA = 0.10
NNUE_SCORE_SCALE = 600.0
# CMake 4 removed the implicit compatibility mode for projects whose pinned
# CMakeLists.txt declares a minimum version below 3.5.  Keep the pinned source
# unchanged and bind the compatibility policy at configure time instead.
R5_CMAKE_POLICY_VERSION_MINIMUM = "3.5"


class R5Error(RuntimeError):
    pass


def fail(message: str) -> None:
    raise R5Error(message)


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        fail(f"cannot read JSON {path}: {exc}")
    if not isinstance(value, dict):
        fail(f"JSON root is not an object: {path}")
    return value


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as stream:
            for block in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(block)
    except OSError as exc:
        fail(f"cannot hash {path}: {exc}")
    return digest.hexdigest()


def _replace_once(path: Path, old: str, new: str, label: str, applied: list[str]) -> None:
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as exc:
        fail(f"cannot read parity compatibility source {path}: {exc}")
    count = text.count(old)
    if count != 1:
        fail(f"parity compatibility guard for {label} expected one match in {path}, found {count}")
    try:
        path.write_text(text.replace(old, new, 1), encoding="utf-8")
    except OSError as exc:
        fail(f"cannot write parity compatibility source {path}: {exc}")
    applied.append(label)


def _patch_probe(context: Path, patch_name: str) -> dict[str, Any]:
    command = ["patch", "--batch", "--fuzz=3", "-p1", "-i", str(ROOT / "trainer-patches" / patch_name)]
    try:
        result = subprocess.run(command, cwd=context, text=True, capture_output=True, check=False)
    except OSError as exc:
        fail(f"cannot run parity patch probe for {patch_name}: {exc}")
    return {
        "patch": patch_name,
        "command": command,
        "returnCode": result.returncode,
        "stdout": result.stdout,
        "stderr": result.stderr,
    }


def _patch_rejects(context: Path) -> list[dict[str, str]]:
    rejects = []
    for path in sorted(context.rglob("*.rej")):
        try:
            content = path.read_text(encoding="utf-8")
        except OSError as exc:
            fail(f"cannot preserve patch reject {path}: {exc}")
        rejects.append({
            "path": str(path.relative_to(context)),
            "sha256": hashlib.sha256(content.encode("utf-8")).hexdigest(),
            "content": content,
        })
    return rejects


def _patch_tool_version() -> str:
    try:
        result = subprocess.run(["patch", "--version"], text=True, capture_output=True, check=False)
    except OSError as exc:
        fail(f"cannot inspect patch tool for parity compatibility: {exc}")
    return (result.stdout or result.stderr).splitlines()[0] if (result.stdout or result.stderr) else "unknown"


def _replay_parity_patch(context: Path, archive: Path, external: Path, receipt_path: Path | None) -> None:
    """Apply the pinned trainer patch contents with exact guards in a disposable parity context.

    Apple patch 2.0 rejects valid hunks from the frozen patch bundle.  The probe
    is retained in the receipt, while this replay applies the exact patch text
    only after the pinned revision and both patch hashes have been verified.
    """
    phase_patch = ROOT / "trainer-patches" / "variant-nnue-pytorch-phase7.1.patch"
    r5_patch = ROOT / "trainer-patches" / "variant-nnue-pytorch-r5.patch"
    if not phase_patch.is_file() or not r5_patch.is_file():
        fail("parity compatibility patches are missing")

    probe = context.parent / "patch-probe"
    probe.mkdir()
    run(["tar", "-xf", str(archive), "-C", str(probe)], context.parent, "extract parity patch probe")
    patch_logs = [_patch_probe(probe, phase_patch.name), _patch_probe(probe, r5_patch.name)]
    rejects = _patch_rejects(probe)

    applied: list[str] = []
    model = context / "model.py"
    train = context / "train.py"
    source_before = {name: sha256(context / name) for name in ("model.py", "train.py")}

    _replace_once(
        model,
        "import ranger\n",
        "import os\nimport ranger\n",
        "R5 model os import",
        applied,
    )
    _replace_once(
        model,
        "# 3 layer fully connected network\nL1 = 512\nL2 = 16\nL3 = 32\n",
        """# 3 layer fully connected network
def _width(name, default):
  value = int(os.environ.get(name, default))
  if value <= 0:
    raise ValueError('{} must be positive'.format(name))
  return value

L1 = _width('HAITAKA_NNUE_L1', 512)
L2 = _width('HAITAKA_NNUE_L2', 16)
L3 = _width('HAITAKA_NNUE_L3', 32)

# This is intentionally a process-level identity.  The serializer and the
# runtime format must be built with the same profile; a model cannot silently
# change width after import.
WIDTH_PROFILE = '{}x{}x{}'.format(L1, L2, L3)
""",
        "R5 dynamic-width model block",
        applied,
    )
    _replace_once(
        model,
        "  def __init__(self, feature_set, lambda_=1.0):\n    super(NNUE, self).__init__()\n",
        "  def __init__(self, feature_set, lambda_=1.0, learning_rate=1.5e-3):\n    super(NNUE, self).__init__()\n    self.learning_rate = learning_rate\n",
        "Phase 7.1 learning-rate constructor",
        applied,
    )
    _replace_once(
        model,
        "  def step_(self, batch, batch_idx, loss_type):\n",
        """  def loss_value(self, batch):
    us, them, white_indices, white_values, black_indices, black_values, outcome, score, psqt_indices, layer_stack_indices = batch
    q = (self(us, them, white_indices, white_values, black_indices, black_values, psqt_indices, layer_stack_indices) * 600 / 361).sigmoid()
    p = (score / 410).sigmoid()
    loss_eval = (p - q).square().mean()
    loss_result = (q - outcome).square().mean()
    return self.lambda_ * loss_eval + (1.0 - self.lambda_) * loss_result

  def step_(self, batch, batch_idx, loss_type):
""",
        "Phase 7.1 loss-value method",
        applied,
    )
    _replace_once(
        model,
        """    us, them, white_indices, white_values, black_indices, black_values, outcome, score, psqt_indices, layer_stack_indices = batch

    # 600 is the kPonanzaConstant scaling factor needed to convert the training net output to a score.
    # This needs to match the value used in the serializer
    nnue2score = 600
    in_scaling = 410
    out_scaling = 361

    q = (self(us, them, white_indices, white_values, black_indices, black_values, psqt_indices, layer_stack_indices) * nnue2score / out_scaling).sigmoid()
    t = outcome
    p = (score / in_scaling).sigmoid()

    loss_eval = (p - q).square().mean()
    loss_result = (q - t).square().mean()
    loss = self.lambda_ * loss_eval + (1.0 - self.lambda_) * loss_result
""",
        "    loss = self.loss_value(batch)\n",
        "Phase 7.1 loss-value step replacement",
        applied,
    )
    _replace_once(
        model,
        "  def validation_step(self, batch, batch_idx):\n    self.step_(batch, batch_idx, 'val_loss')\n",
        "  def validation_step(self, batch, batch_idx, dataloader_idx=0):\n    self.step_(batch, batch_idx, 'id_val_loss' if dataloader_idx == 0 else 'ood_val_loss')\n",
        "Phase 7.1 validation-step compatibility replay",
        applied,
    )
    _replace_once(model, "    LR = 1.5e-3\n", "    LR = self.learning_rate\n", "Phase 7.1 learning-rate optimizer", applied)

    _replace_once(
        train,
        "def make_data_loaders(train_filename, val_filename, feature_set, num_workers, batch_size, filtered, random_fen_skipping, main_device, epoch_size, val_size):\n",
        "def make_data_loaders(train_filename, val_filename, feature_set, num_workers, batch_size, filtered, random_fen_skipping, main_device, epoch_size, val_size, ood_filename=None):\n",
        "Phase 7.1 OOD loader signature",
        applied,
    )
    _replace_once(
        train,
        """  val = DataLoader(nnue_dataset.FixedNumBatchesDataset(val_infinite, (val_size + batch_size - 1) // batch_size), batch_size=None, batch_sampler=None)
  return train, val
""",
        """  val = DataLoader(nnue_dataset.FixedNumBatchesDataset(val_infinite, (val_size + batch_size - 1) // batch_size), batch_size=None, batch_sampler=None)
  if ood_filename is None:
    return train, val
  ood_infinite = nnue_dataset.SparseBatchDataset(features_name, ood_filename, batch_size, filtered=filtered, random_fen_skipping=random_fen_skipping, device=main_device)
  ood = DataLoader(nnue_dataset.FixedNumBatchesDataset(ood_infinite, (val_size + batch_size - 1) // batch_size), batch_size=None, batch_sampler=None)
  return train, [val, ood]

class DiagnosticLogCallback(pl.Callback):
  def __init__(self, batch_size):
    self.batch_size = batch_size

  def on_validation_epoch_end(self, trainer, pl_module):
    if trainer.logger is None or not trainer.optimizers:
      return
    trainer.logger.log_metrics({'optimizer_step': trainer.global_step,
                                'accepted_positions': trainer.global_step * self.batch_size,
                                'learning_rate': trainer.optimizers[0].param_groups[0]['lr']},
                               step=trainer.global_step)
""",
        "Phase 7.1 OOD loader and diagnostic callback",
        applied,
    )
    _replace_once(
        train,
        "  parser.add_argument(\"--validation-size\", type=int, default=1000000, dest='validation_size', help=\"Number of positions per validation step.\")\n",
        """  parser.add_argument("--validation-size", type=int, default=1000000, dest='validation_size', help="Number of positions per validation step.")
  parser.add_argument("--initial-learning-rate", type=float, default=1.5e-3, dest='initial_learning_rate')
  parser.add_argument("--checkpoint-interval-steps", type=int, default=None, dest='checkpoint_interval_steps')
  parser.add_argument("--validation-interval-steps", type=int, default=None, dest='validation_interval_steps')
  parser.add_argument("--max-steps", type=int, default=-1, dest='max_steps')
  parser.add_argument("--ood-validation", default=None, dest='ood_validation')
""",
        "Phase 7.1 training CLI options",
        applied,
    )
    _replace_once(
        train,
        "    nnue = M.NNUE(feature_set=feature_set, lambda_=args.lambda_)\n",
        "    nnue = M.NNUE(feature_set=feature_set, lambda_=args.lambda_, learning_rate=args.initial_learning_rate)\n",
        "Phase 7.1 initial learning rate",
        applied,
    )
    _replace_once(
        train,
        "    nnue.lambda_ = args.lambda_\n",
        "    nnue.lambda_ = args.lambda_\n    nnue.learning_rate = args.initial_learning_rate\n",
        "Phase 7.1 resumed learning rate",
        applied,
    )
    _replace_once(
        train,
        """  checkpoint_callback = pl.callbacks.ModelCheckpoint(save_last=True, every_n_epochs=1, save_top_k=-1)
  trainer = pl.Trainer.from_argparse_args(args, callbacks=[checkpoint_callback], logger=tb_logger)
""",
        """  if args.checkpoint_interval_steps is None:
    checkpoint_callback = pl.callbacks.ModelCheckpoint(save_last=True, every_n_epochs=1, save_top_k=-1)
  else:
    checkpoint_callback = pl.callbacks.ModelCheckpoint(save_last=True, every_n_epochs=None, every_n_train_steps=args.checkpoint_interval_steps, save_top_k=-1)
  if args.validation_interval_steps is not None:
    args.val_check_interval = args.validation_interval_steps
  trainer = pl.Trainer.from_argparse_args(args, callbacks=[checkpoint_callback, DiagnosticLogCallback(batch_size)], logger=tb_logger)
""",
        "Phase 7.1 step checkpointing",
        applied,
    )
    _replace_once(
        train,
        "  train, val = make_data_loaders(args.train, args.val, feature_set, args.num_workers, batch_size, not args.no_smart_fen_skipping, args.random_fen_skipping, main_device, args.epoch_size, args.validation_size)\n",
        "  train, val = make_data_loaders(args.train, args.val, feature_set, args.num_workers, batch_size, not args.no_smart_fen_skipping, args.random_fen_skipping, main_device, args.epoch_size, args.validation_size, args.ood_validation)\n",
        "Phase 7.1 OOD loader wiring",
        applied,
    )
    _replace_once(
        train,
        "  trainer.fit(nnue, train, val)\n",
        "  trainer.fit(nnue, train_dataloaders=train, val_dataloaders=val)\n",
        "Phase 7.1 multi-loader fit",
        applied,
    )

    resulting_hashes = {name: sha256(context / name) for name in ("model.py", "train.py")}
    if receipt_path is not None and not receipt_path.exists():
        receipt_path.parent.mkdir(parents=True, exist_ok=True)
        receipt = {
            "schema": "haitaka-anhoku-step12-parity-patch-compatibility-v1",
            "status": "passed",
            "parityOnly": True,
            "pinnedRevision": PINNED_TRAINER,
            "patchTool": {"command": "patch --version", "version": _patch_tool_version()},
            "patchFiles": [
                {"path": str(phase_patch.relative_to(ROOT)), "sha256": sha256(phase_patch)},
                {"path": str(r5_patch.relative_to(ROOT)), "sha256": sha256(r5_patch)},
            ],
            "sourceBeforeReplay": source_before,
            "originalPatchCommandLogs": patch_logs,
            "preservedRejects": rejects,
            "exactReplacement": {
                "old": "def validation_step(self, batch, batch_idx) -> validation_step(self, batch, batch_idx, dataloader_idx=0)",
                "new": "id_val_loss for dataloader_idx 0; ood_val_loss otherwise",
            },
            "replayedHunks": applied,
            "resultingTemporarySourceHashes": resulting_hashes,
            "adapter": "r5_trainer.prepare_context guarded parity-only textual replay",
            "additionalOptimizerUpdates": 0,
            "checkpointModified": False,
            "sourceCheckout": str(external),
        }
        receipt_path.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def run(command: list[str], cwd: Path, label: str) -> None:
    try:
        subprocess.run(command, cwd=cwd, check=True)
    except (OSError, subprocess.CalledProcessError) as exc:
        fail(f"{label} failed: {exc}")


def workspace_path(value: str) -> Path:
    path = Path(value)
    if path.is_absolute():
        return path
    return ROOT / path


def verify_preflight(report_path: Path, contract_path: Path) -> dict[str, Any]:
    report = load_json(report_path)
    contract = load_json(contract_path)
    if report.get("schema") != "haitaka-anhoku-r5-preflight-report-v1":
        fail("wrong R5 preflight report schema")
    if report.get("readyForTraining") is not True or report.get("trainingStarted") is not False:
        fail("R5 preflight is not a fresh ready-for-training handoff")
    if report.get("r5Authorized") is not False or report.get("passed") is not False:
        fail("preflight report incorrectly claims an R5 decision")
    expected_contract_hash = sha256(contract_path)
    if report.get("contract", {}).get("sha256") != expected_contract_hash:
        fail("preflight report does not bind the supplied R5 contract")
    if contract.get("trainer", {}).get("pinnedRevision") != PINNED_TRAINER:
        fail("R5 contract trainer revision is not the pinned revision")
    if contract.get("optimization", {}).get("batchSize") != BATCH_SIZE:
        fail("R5 trainer only supports the frozen batch size")
    if contract.get("optimization", {}).get("randomFenSkipping") != 0:
        fail("R5 trainer refuses random FEN skipping")
    if contract.get("optimization", {}).get("filtered") is not False:
        fail("R5 trainer refuses filtered input")
    return contract


def verified_binary(record: dict[str, Any], records: int, label: str) -> Path:
    """Check the exact bytes that this process is about to consume."""
    try:
        path = workspace_path(record["path"])
        expected_bytes = record["bytes"]
        expected_hash = record["sha256"]
        actual_bytes = path.stat().st_size
    except (KeyError, TypeError, OSError) as exc:
        fail(f"cannot verify {label} binary: {exc}")
    if expected_bytes != records * ENTRY_BYTES or actual_bytes != expected_bytes:
        fail(f"{label} binary size does not match its frozen identity")
    if sha256(path) != expected_hash:
        fail(f"{label} binary hash does not match its frozen identity")
    return path


def record_training_completion(
    output: Path, checkpoint: Path, stop_reason: str, updates: int, examples_seen: int,
) -> None:
    """Persist termination evidence before CPU conversion or export can fail."""
    if stop_reason not in {"development-plateau", "maximum-updates"}:
        fail(f"unknown training stop reason: {stop_reason}")
    record = {
        "schema": "haitaka-anhoku-r5-training-complete-v1",
        "stopReason": stop_reason,
        "checkpointSha256": sha256(checkpoint),
        "optimizerUpdates": updates,
        "examplesSeen": examples_seen,
    }
    path = output / "training-complete.json"
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temporary.write_text(json.dumps(record, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def write_variant(context: Path) -> None:
    (context / "variant.py").write_text(
        '''RANKS = 9
FILES = 9
SQUARES = RANKS * FILES
KING_SQUARES = RANKS * FILES
PIECE_TYPES = 10
PIECES = 2 * PIECE_TYPES
USE_POCKETS = True
POCKETS = 2 * FILES if USE_POCKETS else 0
RULESET = "anhoku"
DONOR_MODE = "single-front"

PIECE_VALUES = {
    1: 700,
    2: 800,
    3: 400,
    4: 1000,
    5: 100,
    6: 300,
    7: 300,
    8: 500,
    9: 900,
}
''',
        encoding="utf-8",
    )
    (context / "variant.h").write_text(
        """#define FILES 9
#define RANKS 9
#define PIECE_TYPES 10
#define PIECE_COUNT 40
#define POCKETS true
#define KING_SQUARES FILES * RANKS
#define DATA_SIZE 512
#define HAITAKA_DONOR_MODE 2
""",
        encoding="utf-8",
    )


def prepare_context(
    width: dict[str, int],
    cmake: str,
    *,
    parity_only: bool = False,
    compatibility_receipt: Path | None = None,
) -> tuple[tempfile.TemporaryDirectory[str], Path]:
    external = (ROOT / "../engine/variant-nnue-pytorch").resolve()
    if not external.is_dir():
        fail(f"pinned trainer checkout is missing: {external}")
    try:
        revision = subprocess.check_output(
            ["git", "-C", str(external), "rev-parse", "HEAD"], text=True
        ).strip()
    except (OSError, subprocess.CalledProcessError) as exc:
        fail(f"cannot inspect trainer checkout: {exc}")
    if revision != PINNED_TRAINER:
        fail(f"trainer checkout is {revision}, expected pinned {PINNED_TRAINER}")

    temp = tempfile.TemporaryDirectory(prefix="haitaka-r5-trainer-")
    root = Path(temp.name)
    context = root / "context"
    context.mkdir()
    archive = root / "trainer.tar"
    try:
        with archive.open("wb") as stream:
            subprocess.run(
                ["git", "-C", str(external), "archive", "--format=tar", PINNED_TRAINER],
                stdout=stream,
                check=True,
            )
    except (OSError, subprocess.CalledProcessError) as exc:
        temp.cleanup()
        fail(f"cannot archive pinned trainer: {exc}")
    run(["tar", "-xf", str(archive), "-C", str(context)], root, "extract pinned trainer")

    if parity_only:
        _replay_parity_patch(context, archive, external, compatibility_receipt)
    else:
        for patch_name, label in (
            ("variant-nnue-pytorch-phase7.1.patch", "apply Phase 7.1 trainer patch"),
            ("variant-nnue-pytorch-r5.patch", "apply R5 trainer patch"),
        ):
            run(
                ["patch", "--batch", "--fuzz=3", "-p1", "-i", str(ROOT / "trainer-patches" / patch_name)],
                context,
                label,
            )
    trainer_overlay = ROOT / "haitaka_learn" / "trainer_overlay"
    for name in ("feature_set.py", "features.py", "donor_features.py", "training_data_loader.cpp"):
        shutil.copy2(trainer_overlay / name, context / name)
    shutil.copy2(ROOT / "trainer-patches" / "evaluate.py", context / "evaluate.py")
    write_variant(context)
    os.environ.update({
        "HAITAKA_NNUE_L1": str(width["l1"]),
        "HAITAKA_NNUE_L2": str(width["l2"]),
        "HAITAKA_NNUE_L3": str(width["l3"]),
    })
    run(cmake_configure_argv(cmake), context, "configure R5 data loader")
    run([cmake, "--build", "build"], context, "build R5 data loader")
    run([cmake, "--install", "build", "--prefix", "."], context, "install R5 data loader")
    return temp, context


def cmake_configure_argv(cmake: str) -> list[str]:
    """Return the hash-bound configure command for the disposable context."""
    return [
        cmake,
        "-S",
        ".",
        "-B",
        "build",
        f"-DCMAKE_POLICY_VERSION_MINIMUM={R5_CMAKE_POLICY_VERSION_MINIMUM}",
    ]


def import_trainer(context: Path):
    os.chdir(context)
    sys.path.insert(0, str(context))
    import torch  # type: ignore
    import features  # type: ignore
    import model as model_module  # type: ignore
    import nnue_dataset  # type: ignore

    return torch, features, model_module, nnue_dataset


def finite_batches(nnue_dataset, filename: Path, records: int, device) -> Iterable[tuple[Any, ...]]:
    dataset = nnue_dataset.SparseBatchDataset(
        FEATURE_SET,
        str(filename),
        BATCH_SIZE,
        cyclic=False,
        num_workers=1,
        filtered=False,
        random_fen_skipping=0,
        device=device,
    )
    seen = 0
    for batch in dataset:
        size = int(batch[0].shape[0])
        if size <= 0 or seen + size > records:
            fail(f"finite loader overran {filename}: {seen}+{size}>{records}")
        seen += size
        yield batch
    if seen != records:
        fail(f"finite loader consumed {seen} records from {filename}, expected {records}")


def robust_loss(torch, model, batch):
    us, them, white, white_values, black, black_values, _outcome, score, psqt, stacks = batch
    raw = model(us, them, white, white_values, black, black_values, psqt, stacks)
    prediction = raw * NNUE_SCORE_SCALE
    target = score.clamp(-SCORE_CLIP, SCORE_CLIP)
    error = (prediction - target) / SCORE_CLIP
    absolute = error.abs()
    loss = torch.where(
        absolute <= HUBER_DELTA,
        0.5 * error.square(),
        HUBER_DELTA * (absolute - 0.5 * HUBER_DELTA),
    )
    return loss.mean(), prediction, target


def optimizer_for(torch, model, learning_rate: float):
    configured = model.configure_optimizers()
    optimizer = configured[0][0] if isinstance(configured[0], list) else configured[0]
    if not hasattr(optimizer, "param_groups"):
        fail("pinned trainer returned an unexpected optimizer")
    set_learning_rate(optimizer, learning_rate, 0)
    return optimizer


def set_learning_rate(optimizer, base: float, update: int) -> float:
    warmup = 16
    step_updates = 128
    gamma = 0.5
    minimum = 0.00005
    if update < warmup:
        value = base * (update + 1) / warmup
    else:
        value = max(minimum, base * gamma ** ((update - warmup) // step_updates))
    for index, group in enumerate(optimizer.param_groups):
        group["lr"] = value / 10.0 if index >= len(optimizer.param_groups) - 2 else value
    return value


def average_ranks(values: np.ndarray) -> np.ndarray:
    """Assign the mean zero-based rank to every group of equal values."""
    _, inverse, counts = np.unique(values, return_inverse=True, return_counts=True)
    ends = np.cumsum(counts)
    starts = ends - counts
    return ((starts + ends - 1) / 2.0)[inverse]


def metrics_from_predictions(np_predictions: np.ndarray, np_targets: np.ndarray) -> dict[str, Any]:
    error = (np_predictions - np_targets) / SCORE_CLIP
    absolute = np.abs(error)
    huber = np.where(
        absolute <= HUBER_DELTA,
        0.5 * error * error,
        HUBER_DELTA * (absolute - 0.5 * HUBER_DELTA),
    )
    if len(np_predictions) > 1 and np.std(np_predictions) > 0 and np.std(np_targets) > 0:
        pearson = float(np.corrcoef(np_predictions, np_targets)[0, 1])
        prediction_rank = average_ranks(np_predictions)
        target_rank = average_ranks(np_targets)
        spearman = float(np.corrcoef(prediction_rank, target_rank)[0, 1])
    else:
        pearson = None
        spearman = None
    outside = np.abs(np_targets) > 100.0
    return {
        "records": int(len(np_targets)),
        "teacher_only_loss": float(np.mean(huber)),
        "teacher_only_mae": float(np.mean(absolute)),
        "raw_score_mae": float(np.mean(np.abs(np_predictions - np_targets))),
        "pearson": pearson,
        "spearman": spearman,
        "sign_accuracy_outside_near_zero": float(
            np.mean(np.sign(np_predictions[outside]) == np.sign(np_targets[outside]))
        ) if np.any(outside) else None,
        "near_zero_band": 100.0,
        "prediction_activation_rate": float(np.mean(np.abs(np_predictions) > 1.0)),
        "prediction_clipping_rate": float(np.mean(np.abs(np_predictions) >= SCORE_CLIP)),
    }


def evaluate(torch, model, nnue_dataset, filename: Path, records: int, device) -> dict[str, Any]:
    predictions: list[np.ndarray] = []
    targets: list[np.ndarray] = []
    active_rows: set[int] = set()
    model.eval()
    with torch.no_grad():
        for batch in finite_batches(nnue_dataset, filename, records, device):
            _loss, prediction, target = robust_loss(torch, model, batch)
            predictions.append(prediction.detach().cpu().numpy()[:, 0])
            targets.append(target.detach().cpu().numpy()[:, 0])
            for indices in (batch[2], batch[4]):
                active_rows.update(int(value) for value in indices.detach().cpu().numpy().ravel() if value >= 0)
    prediction = np.concatenate(predictions).astype(np.float64, copy=False)
    target = np.concatenate(targets).astype(np.float64, copy=False)
    result = metrics_from_predictions(prediction, target)
    result["active_rows"] = len(active_rows)
    result["prediction_sha256"] = hashlib.sha256(prediction.astype("<f8").tobytes()).hexdigest()
    return result


def seed_everything(torch, seed: int) -> None:
    random.seed(seed)
    np.random.seed(seed)
    torch.manual_seed(seed)
    if torch.cuda.is_available():
        torch.cuda.manual_seed_all(seed)
    torch.use_deterministic_algorithms(True)
    torch.backends.cudnn.benchmark = False
    torch.backends.cudnn.deterministic = True
    torch.set_num_threads(1)


def checkpoint_model(torch, model, path: Path) -> str:
    torch.save(model, path)
    return sha256(path)


def tensor_devices(torch, root: Any) -> set[str]:
    """Return every torch device reachable from a trusted checkpoint object."""
    devices: set[str] = set()
    pending = [root]
    visited: set[int] = set()
    while pending:
        value = pending.pop()
        identity = id(value)
        if identity in visited:
            continue
        visited.add(identity)
        if torch.is_tensor(value):
            devices.add(str(value.device))
        elif isinstance(value, dict):
            pending.extend(value.keys())
            pending.extend(value.values())
        elif isinstance(value, (list, tuple, set, frozenset)):
            pending.extend(value)
        elif hasattr(value, "__dict__"):
            pending.extend(vars(value).values())
    return devices


def materialize_cpu_checkpoint(torch, source: Path, target: Path) -> str:
    """Atomically write and verify a portable CPU copy for serialize.py."""
    temporary = target.with_name(f".{target.name}.{os.getpid()}.tmp")
    try:
        model = torch.load(source, map_location="cpu", weights_only=False)
        model.cpu()
        devices = tensor_devices(torch, model)
        if devices != {"cpu"}:
            fail(f"CPU checkpoint conversion retained unexpected tensor devices: {sorted(devices)}")
        torch.save(model, temporary)

        # Reload without map_location to validate the device tags persisted in
        # the artifact that will be handed to the unchanged pinned serializer.
        persisted = torch.load(temporary, weights_only=False)
        persisted_devices = tensor_devices(torch, persisted)
        if persisted_devices != {"cpu"}:
            fail(
                "persisted CPU checkpoint has unexpected tensor devices: "
                f"{sorted(persisted_devices)}"
            )
        os.replace(temporary, target)
    except (OSError, RuntimeError, TypeError, ValueError) as exc:
        fail(f"cannot materialize CPU checkpoint {target}: {exc}")
    finally:
        temporary.unlink(missing_ok=True)
    return sha256(target)


def train_static_clone(torch, model_module, nnue_dataset, model, optimizer, static: dict[str, Any], device, output: Path, learning_rate: float) -> dict[str, Any]:
    records = static["records"]
    binary = verified_binary(static["binary"], records, "static-clone")
    steps = static["pretrainingUpdates"]
    model.train()
    step = 0
    seen = 0
    losses: list[float] = []
    while step < steps:
        for batch in finite_batches(nnue_dataset, binary, records, device):
            if step >= steps:
                break
            model._clip_weights()
            set_learning_rate(optimizer, learning_rate, step)
            optimizer.zero_grad(set_to_none=True)
            loss, _prediction, _target = robust_loss(torch, model, batch)
            loss.backward()
            optimizer.step()
            step += 1
            seen += int(batch[0].shape[0])
            losses.append(float(loss.detach().cpu()))
    model.eval()
    output.mkdir(parents=True, exist_ok=True)
    report = {
        "schema": "haitaka-anhoku-r5-static-clone-pretraining-v1",
        "recordsPerPass": records,
        "optimizerUpdates": step,
        "examplesSeen": seen,
        "learningRate": learning_rate,
        "teacherOnlyLoss": float(np.mean(losses)),
        "source": static["binary"],
    }
    (output / "static-pretraining.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return report


def run_lane(
    torch,
    model_module,
    features,
    nnue_dataset,
    contract: dict[str, Any],
    lane: dict[str, Any],
    learning_rate: float,
    seed: int,
    width: dict[str, int],
    device,
    output: Path,
) -> dict[str, Any]:
    if output.exists():
        fail(f"fresh R5 recipe refuses existing output directory: {output}")
    optimization = contract["optimization"]
    pilot = contract["pilot"]["splits"]
    train_records = pilot["training"]["records"]
    validation_records = pilot["validation"]["records"]
    development_records = pilot["development"]["records"]
    train_file = verified_binary(pilot["training"]["binary"], train_records, "training")
    validation_file = verified_binary(pilot["validation"]["binary"], validation_records, "validation")
    development_file = verified_binary(pilot["development"]["binary"], development_records, "development")
    if lane["kind"] == "static-clone-full-precision":
        static = lane["staticClone"]
        verified_binary(static["binary"], static["records"], "static-clone")
    output.mkdir(parents=True)

    seed_everything(torch, seed)
    feature_set = features.get_feature_set_from_name(FEATURE_SET)
    model = model_module.NNUE(feature_set=feature_set, lambda_=1.0).to(device)
    optimizer = optimizer_for(torch, model, learning_rate)

    initial_validation = evaluate(torch, model, nnue_dataset, validation_file, validation_records, device)
    initial_development = evaluate(torch, model, nnue_dataset, development_file, development_records, device)
    (output / "untrained-initialization.json").write_text(
        json.dumps(
            {
                "schema": "haitaka-anhoku-r5-untrained-initialization-v1",
                "validation": initial_validation,
                "development": initial_development,
            },
            indent=2,
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )

    static_report = None
    if lane["kind"] == "static-clone-full-precision":
        static_report = train_static_clone(
            torch,
            model_module,
            nnue_dataset,
            model,
            optimizer,
            lane["staticClone"],
            device,
            output,
            learning_rate,
        )
        optimizer = optimizer_for(torch, model, learning_rate)

    checkpoints = output / "checkpoints"
    checkpoints.mkdir()
    checkpoint_path = checkpoints / "last.pt"
    checkpoint_state_path = checkpoints / "last-state.pt"
    checkpoint_rows = output / "checkpoint-metrics.jsonl"
    step = 0
    examples_seen = 0
    best_development = math.inf
    stale_validations = 0
    last_reason = "maximum-updates"
    window_loss = 0.0
    window_mae = 0.0
    window_raw_mae = 0.0
    window_records = 0
    window_active_rows: set[int] = set()
    max_updates = optimization["maximumUpdates"]
    min_updates = optimization["minimumUpdates"]
    validation_every = optimization["validateEveryUpdates"]
    checkpoint_every = optimization["checkpointEveryUpdates"]
    records_per_pass = train_records
    checkpoint_count = 0
    plateau_stop = False
    parameter_count = sum(parameter.numel() for parameter in model.parameters())

    with checkpoint_rows.open("w", encoding="utf-8") as rows:
        while step < max_updates:
            for batch in finite_batches(nnue_dataset, train_file, records_per_pass, device):
                if step >= max_updates:
                    break
                model.train()
                model._clip_weights()
                current_lr = set_learning_rate(optimizer, learning_rate, step)
                optimizer.zero_grad(set_to_none=True)
                loss, prediction, target = robust_loss(torch, model, batch)
                loss.backward()
                gradient_norm = float(torch.nn.utils.clip_grad_norm_(model.parameters(), math.inf).detach().cpu())
                optimizer.step()
                # Project before evaluation and checkpointing. Previously the
                # next iteration did this, leaving checkpoint metrics one
                # update ahead of the effective weights exported by the
                # serializer's clipping path.
                model._clip_weights()
                step += 1
                count = int(batch[0].shape[0])
                examples_seen += count
                window_records += count
                window_loss += float(loss.detach().cpu()) * count
                batch_raw_mae = torch.abs(prediction - target).mean()
                window_raw_mae += float(batch_raw_mae.detach().cpu()) * count
                window_mae += float((batch_raw_mae / SCORE_CLIP).detach().cpu()) * count
                window_active_rows.update(
                    int(value)
                    for indices in (batch[2], batch[4])
                    for value in indices.detach().cpu().numpy().ravel()
                    if value >= 0
                )

                if step % validation_every != 0 and step < max_updates:
                    continue
                if step % checkpoint_every != 0 and step < max_updates:
                    continue
                validation = evaluate(torch, model, nnue_dataset, validation_file, validation_records, device)
                development = evaluate(torch, model, nnue_dataset, development_file, development_records, device)
                model_path_hash = checkpoint_model(torch, model, checkpoint_path)
                state = {
                    "schema": "haitaka-anhoku-r5-checkpoint-state-v1",
                    "lane": lane["id"],
                    "learningRateCandidate": learning_rate,
                    "seed": seed,
                    "width": width,
                    "optimizerUpdates": step,
                    "examplesSeen": examples_seen,
                    "optimizer": optimizer.state_dict(),
                    "model": model.state_dict(),
                    "dataCursor": {
                        "trainRecordsSeen": examples_seen,
                        "equivalentPasses": examples_seen / records_per_pass,
                    },
                    "rng": {
                        "python": repr(random.getstate()),
                        "numpy": repr(np.random.get_state()),
                        "torch": torch.random.get_rng_state().cpu().tolist(),
                        "torchCuda": [state.cpu().tolist() for state in torch.cuda.get_rng_state_all()] if torch.cuda.is_available() else None,
                    },
                }
                torch.save(state, checkpoint_state_path)
                row = {
                    "schema": "haitaka-anhoku-r5-checkpoint-metrics-v1",
                    "lane": lane["id"],
                    "learningRateCandidate": learning_rate,
                    "seed": seed,
                    "width": width,
                    "optimizerUpdates": step,
                    "examplesSeen": examples_seen,
                    "equivalentPasses": examples_seen / records_per_pass,
                    "learningRate": current_lr,
                    "train": {
                        "records": window_records,
                        "teacher_only_loss": window_loss / max(1, window_records),
                        "teacher_only_mae": window_mae / max(1, window_records),
                        "raw_score_mae": window_raw_mae / max(1, window_records),
                        "active_rows": len(window_active_rows),
                        "gradient_norm": gradient_norm,
                    },
                    "validation": validation,
                    "development": development,
                    "checkpoint": {
                        "path": str(checkpoint_path.relative_to(ROOT)),
                        "sha256": model_path_hash,
                        "bytes": checkpoint_path.stat().st_size,
                    },
                    "parameterCount": parameter_count,
                    "serializedBytes": None,
                    "fullRefreshLatency": None,
                    "incrementalLatency": None,
                }
                rows.write(json.dumps(row, sort_keys=True) + "\n")
                rows.flush()
                checkpoint_count += 1
                window_loss = 0.0
                window_mae = 0.0
                window_raw_mae = 0.0
                window_records = 0
                window_active_rows = set()
                development_loss = development["teacher_only_loss"]
                if step >= min_updates:
                    if development_loss < best_development - optimization["plateau"]["minimumDelta"]:
                        best_development = development_loss
                        stale_validations = 0
                    else:
                        stale_validations += 1
                    if stale_validations >= optimization["plateau"]["patienceValidations"]:
                        last_reason = "development-plateau"
                        plateau_stop = True
                        break
            if plateau_stop:
                break
            else:
                continue

    if step < min_updates:
        fail(f"{lane['id']} stopped at {step} updates below minimum {min_updates}")
    record_training_completion(output, checkpoint_path, last_reason, step, examples_seen)
    final_model = checkpoints / "final.pt"
    # Preserve the CUDA-resident resumable checkpoint and give the unchanged
    # pinned serializer a separately verified CPU artifact. Loading with
    # map_location also converts tensor attributes outside model.state_dict().
    final_model_hash = materialize_cpu_checkpoint(torch, checkpoint_path, final_model)
    export = output / "final.nnue"
    run(
        [
            sys.executable,
            "serialize.py",
            str(final_model),
            str(export),
            "--features",
            FEATURE_SET,
            "--description",
            f"Haitaka Anhoku R5 {lane['id']} width {width['l1']}x{width['l2']}x{width['l3']}",
        ],
        Path.cwd(),
        "serialize R5 final network",
    )
    final_validation = evaluate(torch, model, nnue_dataset, validation_file, validation_records, device)
    final_development = evaluate(torch, model, nnue_dataset, development_file, development_records, device)
    report = {
        "schema": "haitaka-anhoku-r5-recipe-run-v1",
        "lane": lane["id"],
        "laneKind": lane["kind"],
        "learningRateCandidate": learning_rate,
        "seed": seed,
        "width": width,
        "optimizerUpdates": step,
        "examplesSeen": examples_seen,
        "equivalentPasses": examples_seen / records_per_pass,
        "stopReason": last_reason,
        "checkpointCount": checkpoint_count,
        "staticPretraining": static_report,
        "finalValidation": final_validation,
        "finalDevelopment": final_development,
        "finalCheckpoint": {
            "path": str(final_model.relative_to(ROOT)),
            "bytes": final_model.stat().st_size,
            "sha256": final_model_hash,
        },
        "exportedInteger": {
            "path": str(export.relative_to(ROOT)),
            "bytes": export.stat().st_size,
            "sha256": sha256(export),
        },
        "finiteExposure": {
            "trainRecordsPerPass": records_per_pass,
            "validationRecordsPerEvaluation": validation_records,
            "developmentRecordsPerEvaluation": development_records,
            "batchSize": BATCH_SIZE,
            "randomFenSkipping": 0,
            "filtered": False,
            "cyclic": False,
            "loaderWorkers": 1,
        },
        "parameterCount": parameter_count,
        "runtimeMetrics": {
            "serializedBytes": export.stat().st_size,
            "fullRefreshLatency": None,
            "incrementalLatency": None,
        },
    }
    (output / "run.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return report


def recover_existing_export(
    torch,
    contract: dict[str, Any],
    model_path: Path,
    output: Path,
    context: Path,
    width: dict[str, int],
) -> dict[str, Any]:
    """Finish a lane whose training/checkpoints completed but export failed.

    This recovery path is deliberately export-only.  It consumes the last
    checkpoint and the already-written final checkpoint metrics; it never
    replays training or changes the frozen data exposure.
    """
    metrics_path = output / "checkpoint-metrics.jsonl"
    if not model_path.is_file() or not metrics_path.is_file():
        fail(f"cannot recover export without checkpoint and metrics: {output}")
    rows = [json.loads(line) for line in metrics_path.read_text(encoding="utf-8").splitlines() if line.strip()]
    if not rows:
        fail(f"cannot recover export from an empty metrics file: {metrics_path}")
    last = rows[-1]
    checkpoint = last.get("checkpoint")
    expected_hash = checkpoint.get("sha256") if isinstance(checkpoint, dict) else None
    if not isinstance(expected_hash, str) or sha256(model_path) != expected_hash:
        fail("recovery checkpoint does not match the final metrics row checkpoint hash")
    if last.get("width") != width:
        fail("recovery width does not match the final metrics row")
    lane_id = last.get("lane")
    lanes = {lane["id"]: lane for lane in contract["initialization"]["lanes"]}
    lane = lanes.get(lane_id)
    if lane is None:
        fail(f"checkpoint metrics contain an unknown lane: {lane_id!r}")

    completion = load_json(output / "training-complete.json")
    expected_completion = {
        "schema": "haitaka-anhoku-r5-training-complete-v1",
        "checkpointSha256": expected_hash,
        "optimizerUpdates": last["optimizerUpdates"],
        "examplesSeen": last["examplesSeen"],
    }
    if any(completion.get(key) != value for key, value in expected_completion.items()):
        fail("training completion record does not match the final metrics row")
    stop_reason = completion.get("stopReason")
    if stop_reason not in {"development-plateau", "maximum-updates"}:
        fail("training completion record has an unknown stop reason")

    final_model = output / "checkpoints" / "final.pt"
    try:
        model = torch.load(model_path, map_location="cpu", weights_only=False)
        model.cpu()
        model.eval()
        torch.save(model, final_model)
    except (OSError, RuntimeError, TypeError) as exc:
        fail(f"cannot materialize CPU export checkpoint: {exc}")
    export = output / "final.nnue"
    run(
        [
            sys.executable,
            "serialize.py",
            str(final_model),
            str(export),
            "--features",
            FEATURE_SET,
            "--description",
            f"Haitaka Anhoku R5 {lane_id} width {width['l1']}x{width['l2']}x{width['l3']}",
        ],
        context,
        "recover R5 final network export",
    )
    static_report_path = output / "static-pretraining.json"
    static_report = load_json(static_report_path) if static_report_path.exists() else None
    report = {
        "schema": "haitaka-anhoku-r5-recipe-run-v1",
        "lane": lane_id,
        "laneKind": lane["kind"],
        "learningRateCandidate": last["learningRateCandidate"],
        "seed": last["seed"],
        "width": width,
        "optimizerUpdates": last["optimizerUpdates"],
        "examplesSeen": last["examplesSeen"],
        "equivalentPasses": last["equivalentPasses"],
        "stopReason": stop_reason,
        "checkpointCount": len(rows),
        "staticPretraining": static_report,
        "finalValidation": last["validation"],
        "finalDevelopment": last["development"],
        "finalCheckpoint": {
            "path": str(final_model.relative_to(ROOT)),
            "bytes": final_model.stat().st_size,
            "sha256": sha256(final_model),
        },
        "exportedInteger": {
            "path": str(export.relative_to(ROOT)),
            "bytes": export.stat().st_size,
            "sha256": sha256(export),
        },
        "finiteExposure": {
            "trainRecordsPerPass": contract["optimization"]["trainRecordsPerPass"],
            "validationRecordsPerEvaluation": contract["optimization"]["validationRecords"],
            "developmentRecordsPerEvaluation": contract["optimization"]["developmentRecords"],
            "batchSize": BATCH_SIZE,
            "randomFenSkipping": 0,
            "filtered": False,
            "cyclic": False,
            "loaderWorkers": 1,
        },
        "recovery": "export-only-from-completed-checkpoint",
    }
    (output / "run.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preflight-report", type=Path, required=True)
    parser.add_argument("--contract", type=Path, default=ROOT / "r0/anhoku-reboot/r5-fixed-data-exposure-contract.json")
    parser.add_argument("--output-dir", type=Path, default=ROOT / "out/anhoku-reboot-r5")
    parser.add_argument("--device", choices=("cuda",), required=True)
    parser.add_argument("--width", choices=("current", "compact"), default="current")
    parser.add_argument("--lane", choices=("scratch", "static-clone-search-finetune", "all"), default="all")
    parser.add_argument("--seed", type=int, default=None)
    parser.add_argument("--learning-rate", type=float, default=None)
    parser.add_argument(
        "--skip-existing",
        action="store_true",
        help="reuse recipe directories that already contain a completed run.json",
    )
    parser.add_argument(
        "--export-existing",
        type=Path,
        default=None,
        help="recover export/report from last.pt and its bound training-complete.json",
    )
    parser.add_argument("--cmake", default="cmake")
    args = parser.parse_args()

    contract_path = args.contract if args.contract.is_absolute() else ROOT / args.contract
    preflight_path = args.preflight_report if args.preflight_report.is_absolute() else ROOT / args.preflight_report
    output_dir = args.output_dir if args.output_dir.is_absolute() else ROOT / args.output_dir
    contract = verify_preflight(preflight_path, contract_path)

    # Importing torch is intentionally delayed until after contract binding and
    # CUDA-only validation has been requested.
    try:
        import torch  # type: ignore
    except ImportError as exc:
        print(f"R5 trainer unavailable: {exc}", file=sys.stderr)
        return 2
    if not torch.cuda.is_available():
        print("R5 trainer requires an approved CUDA device; no GPU is available", file=sys.stderr)
        return 2
    device = torch.device("cuda")

    width = contract["architecture"]["currentWidth" if args.width == "current" else "compactWidth"]
    temp, context = prepare_context(width, args.cmake)
    try:
        torch, features, model_module, nnue_dataset = import_trainer(context)
        if args.export_existing is not None:
            checkpoint = args.export_existing if args.export_existing.is_absolute() else ROOT / args.export_existing
            recipe_output = checkpoint.parent.parent
            report = recover_existing_export(torch, contract, checkpoint, recipe_output, context, width)
            print(f"R5 export recovery written to {recipe_output / 'run.json'}")
            return 0
        initialization = contract["initialization"]
        lanes = initialization["lanes"]
        if args.lane != "all":
            lanes = [lane for lane in lanes if lane["id"] == args.lane]
        rates = contract["optimization"]["initialLearningRateCandidates"]
        if args.learning_rate is not None:
            rates = [args.learning_rate]
        seeds = [args.seed if args.seed is not None else initialization["selectionSeed"]]
        recipe_root = output_dir / ("recipes" if args.width == "current" else "compact-recipes")
        reports = []
        for lane in lanes:
            for rate in rates:
                for seed in seeds:
                    name = f"{lane['id']}-lr-{rate:g}-seed-{seed}"
                    recipe_output = recipe_root / name
                    if recipe_output.exists():
                        if not args.skip_existing or not (recipe_output / "run.json").is_file():
                            fail(f"fresh R5 recipe refuses existing or incomplete output directory: {recipe_output}")
                        reports.append(load_json(recipe_output / "run.json"))
                        continue
                    reports.append(run_lane(
                        torch,
                        model_module,
                        features,
                        nnue_dataset,
                        contract,
                        lane,
                        rate,
                        seed,
                        width,
                        device,
                        recipe_output,
                    ))
        summary = {
            "schema": "haitaka-anhoku-r5-training-summary-v1",
            "width": width,
            "recipeRuns": [
                f"{report['lane']}-lr-{report['learningRateCandidate']:g}-seed-{report['seed']}"
                for report in reports
            ],
            "reports": [
                {
                    "lane": report["lane"],
                    "learningRateCandidate": report["learningRateCandidate"],
                    "seed": report["seed"],
                    "path": str((recipe_root / f"{report['lane']}-lr-{report['learningRateCandidate']:g}-seed-{report['seed']}").relative_to(ROOT)),
                    "runSha256": sha256(recipe_root / f"{report['lane']}-lr-{report['learningRateCandidate']:g}-seed-{report['seed']}" / "run.json"),
                }
                for report in reports
            ],
            "trainingStarted": True,
            "r5DecisionMade": False,
        }
        is_selection_matrix = (
            args.width == "current"
            and args.seed in (None, initialization["selectionSeed"])
            and args.lane == "all"
            and args.learning_rate is None
        )
        if is_selection_matrix:
            summary_name = "training-summary.json"
        elif args.width == "compact" and args.seed in (None, initialization["selectionSeed"]):
            summary_name = "compact-training-summary.json"
        else:
            lane_name = args.lane
            rate_name = "all" if args.learning_rate is None else f"{args.learning_rate:g}"
            seed_name = args.seed if args.seed is not None else initialization["selectionSeed"]
            summary_name = (
                f"replication-{args.width}-{lane_name}-lr-{rate_name}-seed-{seed_name}"
                "-training-summary.json"
            )
        summary_path = output_dir / summary_name
        summary_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        print(f"R5 training summary written to {summary_path}")
    finally:
        temp.cleanup()
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except R5Error as exc:
        print(f"R5 trainer FAILED: {exc}", file=sys.stderr)
        raise SystemExit(1)
