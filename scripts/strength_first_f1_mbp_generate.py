#!/usr/bin/env python3
"""MBP-only F1 proposal pre-generation; local F1 builder applies exclusions.

This does not read outcomes or exclusion sets. It produces exactly the
requested deterministic prefix and leaves every generator receipt intact.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import time


MASTER = {"development": 2026092502, "holdout": 2026092503}


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def artifact(path: Path) -> dict:
    return {"path": str(path), "bytes": path.stat().st_size, "sha256": sha(path)}


def seed(phase: str, index: int) -> int:
    return int.from_bytes(hashlib.sha256(f"f1:{phase}:{MASTER[phase]}:{index}".encode("ascii")).digest()[:8], "big")


def immutable_json(path: Path, value: dict) -> None:
    data = json.dumps(value, indent=2, sort_keys=True, allow_nan=False).encode() + b"\n"
    if path.exists():
        if path.read_bytes() != data:
            raise RuntimeError(f"immutable MBP proposal conflicts: {path}")
        return
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    with temporary.open("xb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, path)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--phase", choices=tuple(MASTER), required=True)
    parser.add_argument("--count", type=int, required=True)
    parser.add_argument("--helper", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if socket.gethostname() != "ITSO-WX6303" or not 1 <= args.count <= 100000:
        raise SystemExit("F1 MBP generation host/count differs")
    helper = args.helper.resolve()
    if not helper.is_file():
        raise SystemExit("F1 MBP generator helper absent")
    output = args.output.resolve()
    started = time.monotonic()
    for index in range(args.count):
        directory = output / args.phase / f"{index:05d}"
        directory.mkdir(parents=True, exist_ok=True)
        suite, report = directory / "proposal.tsv", directory / "generator.json"
        identity = directory / "generation-identity.json"
        identifier = f"f1-{args.phase}-{index:05d}"
        argv = [str(helper), "strength-first-generate-opening-suite", "--output", str(suite), "--report", str(report), "--seed", str(seed(args.phase, index)), "--positions", "1", "--random-plies", "16", "--id-prefix", identifier]
        if "--exclude-suite" in argv:
            raise SystemExit("inner exclusion forbidden")
        if not (suite.exists() and report.exists()):
            completed = subprocess.run(argv, capture_output=True, text=True, timeout=120, check=False)
            if completed.returncode:
                raise SystemExit(f"proposal {args.phase}/{index} failed: {completed.stderr[-500:]}")
        value = {"schema": "haitaka-anhoku-strength-first-f1-mbp-proposal-v1", "phase": args.phase, "proposalIndex": index, "seed": seed(args.phase, index), "producerHost": socket.gethostname(), "producedAtUtc": datetime.now(timezone.utc).isoformat(), "argv": argv, "helper": artifact(helper), "output": artifact(suite), "report": artifact(report), "gpuOccurred": False, "gamesOccurred": False}
        if identity.exists():
            value["producedAtUtc"] = json.loads(identity.read_text())["producedAtUtc"]
        immutable_json(identity, value)
        if (index + 1) % 64 == 0:
            print(json.dumps({"phase": args.phase, "completed": index + 1, "elapsedSeconds": time.monotonic() - started}), flush=True)
    print(json.dumps({"phase": args.phase, "completed": args.count, "elapsedSeconds": time.monotonic() - started}), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
