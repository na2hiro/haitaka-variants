#!/usr/bin/env python3
"""External F1 paid-instance heartbeat and conservative budget monitor.

Run on the local controller while the single approved Vast instance exists.
Each sample queries the owner's Vast CLI and SSH telemetry, then atomically
publishes a current sample on the instance. It never creates an instance.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import math
import os
from pathlib import Path
import shlex
import subprocess
import sys
import time
from typing import Any

import strength_first_f1_core as f1


def _instances() -> list[dict[str, Any]]:
    completed = subprocess.run(["vastai", "show", "instances", "--raw"], capture_output=True, text=True, timeout=30, check=False)
    f1.require(completed.returncode == 0, "Vast owner instance query failed")
    value = json.loads(completed.stdout)
    if isinstance(value, dict):
        value = value.get("instances", value.get("results"))
    f1.require(isinstance(value, list), "Vast owner instance response differs")
    return value


def _hours(row: dict[str, Any]) -> float:
    # Vast uptime_mins can describe the host rather than this owned rental.
    # Bill from the owned instance start and conservatively include its duration.
    started = row.get("start_date")
    f1.require(type(started) in (int, float) and math.isfinite(started) and started > 0, "Vast instance billed start time unavailable")
    elapsed = max(0.0, time.time() - float(started))
    duration = row.get("duration")
    if duration is not None:
        f1.require(type(duration) in (int, float) and math.isfinite(duration) and duration >= 0, "Vast instance duration unavailable")
        elapsed = max(elapsed, float(duration))
    return elapsed / 3600


def _remote_sample(host: str, port: int, remote_root: str) -> dict[str, Any]:
    code = "import json,pathlib,shutil,subprocess\nroot=pathlib.Path(" + repr(remote_root) + ")\n" + """
d=shutil.disk_usage(root)
p=subprocess.run(['nvidia-smi','--query-gpu=memory.used,memory.total,utilization.gpu','--format=csv,noheader,nounits'],capture_output=True,text=True,timeout=8)
memory={line.split(':',1)[0]:int(line.split(':',1)[1].strip().split()[0])*1024 for line in pathlib.Path('/proc/meminfo').read_text().splitlines() if ':' in line}
ram_total=memory.get('MemTotal')
ram_available=memory.get('MemAvailable')
ram_source='host-meminfo'
ram_limit=None
ram_used=None
for maximum,current,source in (('/sys/fs/cgroup/memory.max','/sys/fs/cgroup/memory.current','cgroup-v2'),('/sys/fs/cgroup/memory/memory.limit_in_bytes','/sys/fs/cgroup/memory/memory.usage_in_bytes','cgroup-v1')):
    maximum_path,current_path=pathlib.Path(maximum),pathlib.Path(current)
    if not maximum_path.is_file() or not current_path.is_file():
        continue
    limit_text=maximum_path.read_text().strip()
    if not limit_text.isdecimal():
        continue
    limit,used=int(limit_text),int(current_path.read_text().strip())
    if limit <= 0 or used < 0 or ram_total is None or limit >= ram_total:
        continue
    ram_limit,ram_used=limit,used
    ram_total=min(ram_total,limit)
    ram_available=min(ram_available,max(0,limit-used))
    ram_source=source
    break
updates=list((root/'training'/'arms').glob('*/attempts/*/updates/u*-complete.json'))
last=max(updates,key=lambda path:path.stat().st_mtime_ns) if updates else None
progress=None
if last:
    receipt=json.loads(last.read_text())
    progress={'arm':last.parts[-5],'update':receipt['update'],'completedAtUtc':receipt['completedAtUtc']}
print(json.dumps({'freeDiskBytes':d.free,'gpu':p.stdout.strip(),'gpuQueryStatus':p.returncode,'ramAvailableBytes':ram_available,'ramTotalBytes':ram_total,'ramSource':ram_source,'ramLimitBytes':ram_limit,'ramUsedBytes':ram_used,'lastUpdate':progress}))"""
    completed = subprocess.run(["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", "-p", str(port), f"root@{host}", "python3 -c " + shlex.quote(code)], capture_output=True, text=True, timeout=15, check=False)
    f1.require(completed.returncode == 0, "owned instance SSH/telemetry unavailable")
    value = json.loads(completed.stdout)
    f1.require(value.get("gpuQueryStatus") == 0 and isinstance(value.get("gpu"), str) and value["gpu"], "owned GPU telemetry unavailable")
    f1.require(type(value.get("freeDiskBytes")) is int and value["freeDiskBytes"] >= 16 * 1024**3, "owned instance archival reserve exhausted")
    f1.require(type(value.get("ramAvailableBytes")) is int and value["ramAvailableBytes"] >= 1024**3, "owned instance available RAM below 1 GiB")
    f1.require(type(value.get("ramTotalBytes")) is int and value["ramTotalBytes"] >= value["ramAvailableBytes"] and value.get("ramSource") in ("cgroup-v2", "cgroup-v1", "host-meminfo"), "owned instance RAM telemetry unavailable")
    parts = [part.strip() for part in value["gpu"].splitlines()[0].split(",")]
    f1.require(len(parts) == 3 and all(part.isdecimal() for part in parts), "owned GPU memory/utilization telemetry differs")
    value["gpuMemoryUsedMiB"], value["gpuMemoryTotalMiB"], value["gpuUtilizationPercent"] = map(int, parts)
    f1.require(0 <= value["gpuMemoryUsedMiB"] <= value["gpuMemoryTotalMiB"] and value["gpuMemoryTotalMiB"] >= 22 * 1024 and 0 <= value["gpuUtilizationPercent"] <= 100, "owned GPU telemetry outside approved bounds")
    return value


def sample(root: Path, remote_root: str) -> dict[str, Any]:
    package = json.loads((root / "package.json").read_text())
    grant = json.loads((root / "authorization/rental.json").read_text())
    package_hash = f1.sha256(root / "package.json")
    config_hash = f1.sha256(root / "training/config.json")
    f1.require(package.get("status") == "frozen" and package.get("experimentId") == f1.EXPERIMENT_ID, "F1 package absent")
    f1.require(package.get("config") == f1.artifact(root / "training/config.json"), "F1 package config binding differs")
    execution = json.loads((root / "authorization/execution.json").read_text())
    for role, current in (("execution", execution), ("rental", grant)):
        f1.require(current.get("schema") == f"haitaka-anhoku-strength-first-f1-{role}-grant-v1" and current.get("experimentId") == f1.EXPERIMENT_ID, f"F1 {role} grant identity differs")
        f1.require(current.get("authorized") is True and current.get("userReference") and current.get("packageSha256") == package_hash and current.get("configSha256") == config_hash, f"F1 {role} grant or binding absent")
        f1.require(current.get("inputHashes") == {name: item[2] for name, item in f1.INPUTS.items()}, f"F1 {role} input binding differs")
        expiry = datetime.fromisoformat(current["validUntilUtc"].replace("Z", "+00:00"))
        f1.require(expiry.tzinfo is not None and expiry.astimezone(timezone.utc) > datetime.now(timezone.utc), f"F1 {role} grant expired")
        f1.require(current.get("oneInstance") is True and type(current.get("maximumBilledHours")) in (int, float) and current["maximumBilledHours"] <= 4 and type(current.get("maximumAllInUsd")) in (int, float) and current["maximumAllInUsd"] <= 5, f"F1 {role} lifecycle cap differs")
    f1.require(execution.get("cudaFixturesAuthorized") is True and execution.get("productionTrainingAuthorized") is True and execution.get("arms") == ["C", "T"] and execution.get("updatesPerArm") == 1024, "F1 execution scope differs")
    f1.require(grant.get("terminationAuthorized") is True, "F1 owned shutdown authority absent")
    instance_id = grant.get("instanceId")
    f1.require(type(instance_id) is int and instance_id > 0 and execution.get("instanceId") == instance_id, "owned instance ID differs")
    rows = [row for row in _instances() if row.get("id") == instance_id]
    f1.require(len(rows) == 1, "approved Vast instance unavailable or duplicated")
    row = rows[0]
    status = row.get("actual_status", row.get("status"))
    f1.require(status == "running", "owned Vast instance is not running")
    host, port = row.get("ssh_host"), row.get("ssh_port")
    f1.require(isinstance(host, str) and host and type(port) is int and port > 0, "owned instance SSH endpoint unavailable")
    remote = _remote_sample(host, port, remote_root)
    quote = json.loads((root / "package/quote.json").read_text())
    hours = _hours(row)
    rate = row.get("dph_total")
    f1.require(type(rate) in (int, float) and math.isfinite(rate) and rate >= 0, "actual Vast hourly rate unavailable")
    # Include the full transfer reserve in every live bound, even before it is used.
    transfer = json.loads((root / "package/forecast.json").read_text())["costForecastUsd"]["transferReserveAtQuote"]
    estimated_all_in = hours * max(float(rate), float(quote["offer"]["dph_total"])) + float(transfer)
    f1.require(math.isfinite(hours) and math.isfinite(estimated_all_in) and hours < min(4, execution["maximumBilledHours"], grant["maximumBilledHours"]) and estimated_all_in < min(5, execution["maximumAllInUsd"], grant["maximumAllInUsd"]), "paid lifecycle cap reached")
    value = {"schema": "haitaka-anhoku-strength-first-f1-paid-monitor-v1", "experimentId": f1.EXPERIMENT_ID, "observedAtUtc": f1.utc_now(), "instanceId": instance_id, "status": "running", "sshReachable": True, "billedHours": hours, "allInUsd": estimated_all_in, "allInMethod": "conservative-current-rate-plus-full-transfer-reserve", "hourlyRateUsd": rate, "freeDiskBytes": remote["freeDiskBytes"], "gpu": remote["gpu"], "gpuQueryStatus": remote["gpuQueryStatus"], "gpuMemoryUsedMiB": remote["gpuMemoryUsedMiB"], "gpuMemoryTotalMiB": remote["gpuMemoryTotalMiB"], "gpuUtilizationPercent": remote["gpuUtilizationPercent"], "ramAvailableBytes": remote["ramAvailableBytes"], "ramTotalBytes": remote["ramTotalBytes"], "ramSource": remote["ramSource"], "ramLimitBytes": remote["ramLimitBytes"], "ramUsedBytes": remote["ramUsedBytes"], "lastUpdate": remote["lastUpdate"], "packageSha256": package_hash, "executionGrantSha256": f1.sha256(root / "authorization/execution.json"), "rentalGrantSha256": f1.sha256(root / "authorization/rental.json")}
    local = root / "authorization/monitor.json"
    local.parent.mkdir(parents=True, exist_ok=True)
    temporary = local.with_name(".monitor.json.tmp")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")
    os.replace(temporary, local)
    destination = f"root@{host}:{remote_root}/authorization/monitor.json"
    transferred = subprocess.run(["rsync", "-a", "-e", f"ssh -o BatchMode=yes -p {port}", str(local), destination], capture_output=True, text=True, timeout=20, check=False)
    f1.require(transferred.returncode == 0, "paid monitor heartbeat transfer failed")
    return value


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=f1.F1_ROOT)
    parser.add_argument("--remote-root", default=str(f1.F1_ROOT))
    parser.add_argument("--interval-seconds", type=int, default=20)
    parser.add_argument("--once", action="store_true")
    args = parser.parse_args()
    if not 1 <= args.interval_seconds <= 20:
        parser.error("sample interval must be between 1 and 20 seconds")
    try:
        while True:
            value = sample(args.root.resolve(), args.remote_root)
            print(json.dumps({"observedAtUtc": value["observedAtUtc"], "instanceId": value["instanceId"], "billedHours": value["billedHours"], "allInUsd": value["allInUsd"], "freeDiskBytes": value["freeDiskBytes"], "gpuMemoryUsedMiB": value["gpuMemoryUsedMiB"], "gpuUtilizationPercent": value["gpuUtilizationPercent"], "ramAvailableBytes": value["ramAvailableBytes"], "lastUpdate": value["lastUpdate"]}, sort_keys=True), flush=True)
            if args.once:
                break
            time.sleep(args.interval_seconds)
    except (f1.F1Error, OSError, ValueError, KeyError, TypeError, subprocess.TimeoutExpired) as exc:
        print(f"F1 paid monitor stopped: {exc}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
