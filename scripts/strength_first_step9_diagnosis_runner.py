#!/usr/bin/env python3
"""Unattended serial runner for the fixed Step 9 diagnosis schedule."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time
from typing import Any


BLOCK_GAMES = 256
TOTAL_BLOCKS = 16


class RunnerError(RuntimeError):
    pass


def now() -> str:
    return datetime.now(timezone.utc).isoformat()


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise RunnerError(f"JSON root is not an object: {path}")
    return value


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def resolve(root: Path, value: str) -> Path:
    path = Path(value)
    return path if path.is_absolute() else root / path


def update_status(root: Path, **updates: Any) -> None:
    path = root / "status.json"
    status = load_json(path)
    status.update(updates)
    status["updatedAt"] = now()
    write_json(path, status)


def numeric_attempts(block_dir: Path) -> list[Path]:
    attempts = []
    if not block_dir.is_dir():
        return attempts
    for path in block_dir.glob("attempt-*"):
        if path.is_dir():
            try:
                int(path.name.removeprefix("attempt-"))
            except ValueError:
                continue
            attempts.append(path)
    return sorted(attempts, key=lambda path: int(path.name.removeprefix("attempt-")))


def next_attempt(block_dir: Path) -> Path:
    attempts = numeric_attempts(block_dir)
    number = 1 if not attempts else int(attempts[-1].name.removeprefix("attempt-")) + 1
    target = block_dir / f"attempt-{number}"
    target.mkdir(parents=True, exist_ok=False)
    return target


def validate_existing(root: Path, analyzer: Path, cell: str, block: int, attempt: Path) -> bool:
    validation = attempt / "validation.json"
    if not validation.is_file():
        return False
    command = [sys.executable, str(analyzer), "validate-block", "--root", str(root), "--cell", cell, "--block", str(block), "--attempt-dir", str(attempt)]
    result = subprocess.run(command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, cwd=root)
    return result.returncode == 0


class SerialRunner:
    def __init__(self, root: Path):
        self.root = root.resolve()
        self.manifest = load_json(self.root / "manifest.json")
        self.status = load_json(self.root / "status.json")
        self.current_game: subprocess.Popen[bytes] | None = None
        self.current_cell: str | None = None
        self.current_block: int | None = None
        self.current_attempt: Path | None = None
        self.interrupting = False
        self.caffeinate: subprocess.Popen[bytes] | None = None
        saved_completed = self.status.get("completedBlocks", [])
        if not isinstance(saved_completed, list) or any(not isinstance(item, dict) for item in saved_completed):
            raise RunnerError("status completedBlocks must be a list of objects")
        self.completed: list[dict[str, Any]] = list(saved_completed)
        self.completed_keys = {(item.get("cell"), item.get("block")) for item in self.completed}
        if len(self.completed_keys) != len(self.completed):
            raise RunnerError("status completedBlocks contains duplicate cell/block entries")
        self.started = time.monotonic()

    def verify_bindings(self) -> tuple[Path, Path]:
        if self.manifest.get("schema") != "haitaka-anhoku-strength-first-step9-diagnosis-v1":
            raise RunnerError("wrong diagnosis manifest schema")
        if self.status.get("manifestSha256") != sha256(self.root / "manifest.json"):
            raise RunnerError("status does not bind the manifest hash")
        runner = resolve(self.root, self.manifest["runner"]["path"])
        analyzer = resolve(self.root, self.manifest["analyzer"]["path"])
        if sha256(runner) != self.manifest["runner"]["sha256"]:
            raise RunnerError("runner hash differs from prepared manifest")
        if sha256(analyzer) != self.manifest["analyzer"]["sha256"]:
            raise RunnerError("analyzer hash differs from prepared manifest")
        runtime = resolve(self.root, self.manifest["runtime"]["executed"]["path"])
        if sha256(runtime) != self.manifest["runtime"]["executed"]["sha256"]:
            raise RunnerError("runtime hash differs from prepared manifest")
        return runtime, analyzer

    def handle_signal(self, signum: int, _frame: Any) -> None:
        if self.interrupting:
            return
        self.interrupting = True
        if self.current_game is not None and self.current_game.poll() is None:
            try:
                self.current_game.send_signal(signal.SIGINT)
                self.current_game.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.current_game.terminate()
                try:
                    self.current_game.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    self.current_game.kill()
            except ProcessLookupError:
                pass
        update_status(
            self.root,
            state="interrupted",
            runnerPid=None,
            gamePid=None,
            active=None,
            interrupted={"signal": signum, "cell": self.current_cell, "block": self.current_block, "attempt": self.current_attempt.name if self.current_attempt else None, "partialAttemptPreserved": True},
            completedBlocks=self.completed,
            completedGames=len(self.completed) * BLOCK_GAMES,
        )
        raise KeyboardInterrupt

    def start_caffeinate(self) -> None:
        executable = shutil.which("caffeinate")
        if executable is None:
            raise RunnerError("caffeinate is required on the MBP")
        self.caffeinate = subprocess.Popen([executable, "-dimsu"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def stop_caffeinate(self) -> None:
        if self.caffeinate is not None and self.caffeinate.poll() is None:
            self.caffeinate.terminate()
            try:
                self.caffeinate.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.caffeinate.kill()
        self.caffeinate = None

    def run_block(self, runtime: Path, analyzer: Path, cell: str, block: int, position: int) -> None:
        block_dir = self.root / "matches" / cell / f"block-{block}"
        block_dir.mkdir(parents=True, exist_ok=True)
        key = (cell, block)
        for attempt in numeric_attempts(block_dir):
            if validate_existing(self.root, analyzer, cell, block, attempt):
                if key not in self.completed_keys:
                    record = {"position": position, "cell": cell, "block": block, "attempt": attempt.name, "status": "already-validated"}
                    self.completed.append(record)
                    self.completed_keys.add(key)
                update_status(self.root, state="running", runnerPid=os.getpid(), gamePid=None, active=None, completedBlocks=self.completed, completedGames=len(self.completed) * BLOCK_GAMES)
                print(json.dumps({"status": "passed", "position": position, "cell": cell, "block": block, "completedGames": len(self.completed) * BLOCK_GAMES}), flush=True)
                return
        if key in self.completed_keys:
            raise RunnerError(f"status claims completed block but no valid sealed block exists for {cell} block {block}")
        attempt = next_attempt(block_dir)
        self.current_cell = cell
        self.current_block = block
        self.current_attempt = attempt
        definition = self.manifest["schedule"]["cells"][cell]
        suite = definition["suite"]
        prefix = "a" if suite == "A" else "b"
        block_sfen = self.root / "schedules/blocks" / f"{prefix}-selected-block-{block}.sfen"
        network = resolve(self.root, definition["network"]["path"])
        report_dir = attempt / "native-raw"
        report_dir.mkdir(parents=True, exist_ok=True)
        command = [
            str(runtime), "self-play",
            "--games", str(BLOCK_GAMES), "--threads", "1",
            "--a-eval", "nnue", "--b-eval", "handcrafted", "--a-nnue", str(network),
            "--movetime-ms", "100", "--openings", str(block_sfen), "--opening-order", "sequential",
            "--opening-random-plies", "0", "--seed", str(definition["suiteSeed"]), "--max-plies", "200",
            "--report-dir", str(report_dir),
        ]
        write_json(attempt / "planned-command.json", {"schema": "haitaka-anhoku-strength-first-step9-diagnosis-planned-block-v1", "position": position, "cell": cell, "block": block, "attempt": attempt.name, "command": command, "networkSha256": definition["network"]["sha256"], "suiteSeed": definition["suiteSeed"], "runtimeSha256": self.manifest["runtime"]["executed"]["sha256"]})
        log = (attempt / "native-raw/self-play.log").open("w", encoding="utf-8")
        update_status(self.root, state="running", runnerPid=os.getpid(), gamePid=None, active={"position": position, "cell": cell, "block": block, "attempt": attempt.name}, completedBlocks=self.completed, completedGames=len(self.completed) * BLOCK_GAMES)
        started = time.monotonic()
        try:
            self.current_game = subprocess.Popen(command, cwd=self.root, stdout=log, stderr=subprocess.STDOUT)
            update_status(self.root, state="running", runnerPid=os.getpid(), gamePid=self.current_game.pid, active={"position": position, "cell": cell, "block": block, "attempt": attempt.name}, completedBlocks=self.completed, completedGames=len(self.completed) * BLOCK_GAMES)
            return_code = self.current_game.wait()
        finally:
            log.close()
            self.current_game = None
        elapsed = time.monotonic() - started
        if self.interrupting:
            raise KeyboardInterrupt
        if return_code != 0:
            update_status(self.root, state="failed", runnerPid=None, gamePid=None, active=None, failure={"kind": "game-process-failure", "position": position, "cell": cell, "block": block, "attempt": attempt.name, "returnCode": return_code, "partialAttemptPreserved": True}, completedBlocks=self.completed, completedGames=len(self.completed) * BLOCK_GAMES)
            raise RunnerError(f"game process failed for {cell} block {block}")
        if not (report_dir / "self-play-report.json").is_file() or not (report_dir / "self-play-games.jsonl").is_file():
            update_status(self.root, state="failed", runnerPid=None, gamePid=None, active=None, failure={"kind": "game-artifacts-missing", "position": position, "cell": cell, "block": block, "attempt": attempt.name, "partialAttemptPreserved": True}, completedBlocks=self.completed, completedGames=len(self.completed) * BLOCK_GAMES)
            raise RunnerError(f"game artifacts missing for {cell} block {block}")
        validation_log = (attempt / "validator.log").open("w", encoding="utf-8")
        validation_command = [sys.executable, str(analyzer), "validate-block", "--root", str(self.root), "--cell", cell, "--block", str(block), "--attempt-dir", str(attempt)]
        try:
            validation = subprocess.run(validation_command, cwd=self.root, stdout=subprocess.DEVNULL, stderr=validation_log)
        finally:
            validation_log.close()
        if validation.returncode != 0:
            update_status(self.root, state="failed", runnerPid=None, gamePid=None, active=None, failure={"kind": "block-validation-failure", "position": position, "cell": cell, "block": block, "attempt": attempt.name, "partialAttemptPreserved": True}, completedBlocks=self.completed, completedGames=len(self.completed) * BLOCK_GAMES)
            raise RunnerError(f"block validation failed for {cell} block {block}")
        record = {"position": position, "cell": cell, "block": block, "attempt": attempt.name, "status": "validated", "elapsedSeconds": elapsed}
        self.completed.append(record)
        self.completed_keys.add(key)
        update_status(self.root, state="running", runnerPid=os.getpid(), gamePid=None, active=None, completedBlocks=self.completed, completedGames=len(self.completed) * BLOCK_GAMES, lastBlockElapsedSeconds=elapsed)
        print(json.dumps({"status": "passed", "position": position, "cell": cell, "block": block, "completedGames": len(self.completed) * BLOCK_GAMES, "elapsedSeconds": elapsed}), flush=True)

    def run(self) -> None:
        runtime, analyzer = self.verify_bindings()
        signal.signal(signal.SIGINT, self.handle_signal)
        signal.signal(signal.SIGTERM, self.handle_signal)
        self.start_caffeinate()
        try:
            update_status(self.root, state="running", runnerPid=os.getpid(), gamePid=None, active=None, completedBlocks=self.completed, completedGames=len(self.completed) * BLOCK_GAMES, totalBlocks=TOTAL_BLOCKS, totalGames=TOTAL_BLOCKS * BLOCK_GAMES)
            for item in self.manifest["schedule"]["executionOrder"]:
                self.run_block(runtime, analyzer, item["cell"], int(item["block"]), int(item["position"]))
            if len(self.completed_keys) != TOTAL_BLOCKS:
                raise RunnerError("fixed schedule did not produce 16 validated blocks")
            update_status(self.root, state="complete", runnerPid=None, gamePid=None, active=None, completedBlocks=self.completed, completedGames=TOTAL_BLOCKS * BLOCK_GAMES, totalBlocks=TOTAL_BLOCKS, totalGames=TOTAL_BLOCKS * BLOCK_GAMES, remainingOwnedProcesses=[])
            print(json.dumps({"status": "complete", "completedBlocks": TOTAL_BLOCKS, "completedGames": TOTAL_BLOCKS * BLOCK_GAMES}), flush=True)
        finally:
            self.stop_caffeinate()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True, type=Path)
    args = parser.parse_args()
    try:
        SerialRunner(args.root).run()
        return 0
    except KeyboardInterrupt:
        print(json.dumps({"status": "interrupted"}), file=sys.stderr)
        return 130
    except (RunnerError, OSError, subprocess.SubprocessError) as exc:
        print("runner failed", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
