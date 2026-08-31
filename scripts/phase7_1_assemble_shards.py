#!/usr/bin/env python3
"""Verify Phase 7 shards and assemble the fixed Phase 7.1 ID split.

The input archives are treated as untrusted. Only exact shard-XXXXXX.bin/json
members under train/validation are accepted; AppleDouble and xattr members are
ignored. If a preserved on-disk shard root is supplied, it is reconciled with
the archives by exact bytes before it can fill archive gaps. The command fails
before writing model-input data when the complete 2,500/250 shard gate is not
met.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import random
import re
import shutil
import tarfile
import tempfile
from pathlib import Path


ENTRY_BYTES = 72
EXPECTED = {"train": 2500, "validation": 250}
CONFIG_HASH = "e4e879a255a1ad4d20b665001b7c9e434c7d089ce2813de0ad0cdf3e311f553c"
EXPECTED_ENGINE_REVISION = "3841b5e5ef82836bcc2362b1b1469ca5bf798ff8"
EXPECTED_OPENINGS = {f"anhoku-v1-{i:03d}" for i in range(1, 13) if i not in (10, 11)}
SHARD_RE = re.compile(r"^.+/datasets/shards/(train|validation)/shard-([0-9]{6})\.(bin|json)$")


def splitmix64(value: int) -> int:
    value = (value + 0x9E3779B97F4A7C15) & 0xFFFFFFFFFFFFFFFF
    value = ((value ^ (value >> 30)) * 0xBF58476D1CE4E5B9) & 0xFFFFFFFFFFFFFFFF
    value = ((value ^ (value >> 27)) * 0x94D049BB133111EB) & 0xFFFFFFFFFFFFFFFF
    return value ^ (value >> 31)


def member_kind(name: str):
    if "/._" in name or name.startswith("._") or "/__MACOSX/" in name:
        return None
    match = SHARD_RE.match(name)
    if match is None:
        if "/shard-" in name:
            raise ValueError(f"invalid shard member path: {name}")
        return None
    return match.group(1), int(match.group(2)), match.group(3)


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def source_bytes(source):
    archive_name, member_name = source[:2]
    if archive_name is None:
        return Path(member_name).read_bytes()
    with tarfile.open(archive_name, "r:gz") as stream:
        member = stream.getmember(member_name)
        handle = stream.extractfile(member)
        if handle is None:
            raise ValueError(f"cannot read archive member {member_name}")
        return handle.read()


def source_sha256(source):
    if len(source) >= 3:
        return source[2]
    archive_name, member_name = source[:2]
    digest = hashlib.sha256()
    if archive_name is None:
        with Path(member_name).open("rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
        return digest.hexdigest()
    with tarfile.open(archive_name, "r:gz") as stream:
        member = stream.getmember(member_name)
        handle = stream.extractfile(member)
        if handle is None:
            raise ValueError(f"cannot read archive member {member_name}")
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def add_inventory_entry(inventory, key, source, identical_duplicates):
    previous = inventory.get(key)
    if previous is None:
        inventory[key] = source
        return
    if source_sha256(previous) != source_sha256(source):
        raise ValueError(
            f"conflicting duplicate {key[0]} shard {key[1]:06d} {key[2]}: "
            f"{previous[0] or previous[1]} and {source[0] or source[1]}"
        )
    identical_duplicates.append({
        "split": key[0],
        "shard_id": key[1],
        "extension": key[2],
        "kept": previous[0] or previous[1],
        "duplicate": source[0] or source[1],
    })


def archive_inventory(archives: list[Path]):
    inventory = {}
    ignored = []
    archive_hashes = {}
    identical_duplicates = []
    for archive in archives:
        archive_hashes[str(archive)] = sha256_bytes(archive.read_bytes())
        with tarfile.open(archive, "r:gz") as stream:
            for member in stream:
                kind = member_kind(member.name)
                if kind is None:
                    if "/._" in member.name or member.name.startswith("._"):
                        ignored.append(member.name)
                    continue
                if not member.isfile():
                    raise ValueError(f"shard member is not a regular file: {member.name}")
                split, shard_id, extension = kind
                key = (split, shard_id, extension)
                handle = stream.extractfile(member)
                if handle is None:
                    raise ValueError(f"cannot read archive member {member.name}")
                digest = hashlib.sha256()
                for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                    digest.update(chunk)
                add_inventory_entry(
                    inventory,
                    key,
                    (str(archive), member.name, digest.hexdigest()),
                    identical_duplicates,
                )
    return inventory, ignored, archive_hashes, identical_duplicates


def directory_inventory(root: Path):
    inventory = {}
    partial_candidates = []
    for split in EXPECTED:
        split_root = root / split
        if not split_root.is_dir():
            continue
        for path in sorted(split_root.iterdir()):
            if not path.is_file():
                continue
            match = re.fullmatch(r"shard-([0-9]{6})\.(bin|json)", path.name)
            if match is None:
                if re.fullmatch(r"shard-[0-9]{6}\.bin\.tmp", path.name):
                    partial_candidates.append({"path": str(path), "bytes": path.stat().st_size})
                continue
            key = (split, int(match.group(1)), match.group(2))
            add_inventory_entry(
                inventory,
                key,
                (None, str(path), source_sha256((None, str(path)))),
                [],
            )
    return inventory, partial_candidates


def read_member(inventory, key) -> bytes:
    return source_bytes(inventory[key])


def load_shard(inventory, split: str, shard_id: int):
    manifest = json.loads(read_member(inventory, (split, shard_id, "json")))
    binary = read_member(inventory, (split, shard_id, "bin"))
    if manifest.get("config_hash") != CONFIG_HASH:
        raise ValueError(f"{split} shard {shard_id:06d} has a mismatched config hash")
    if manifest.get("entry_bytes") != ENTRY_BYTES:
        raise ValueError(f"{split} shard {shard_id:06d} has a mismatched ABI")
    expected_bytes = manifest.get("sampled_positions", 0) * ENTRY_BYTES
    if len(binary) != expected_bytes:
        raise ValueError(
            f"{split} shard {shard_id:06d} has {len(binary)} bytes; "
            f"expected {expected_bytes}"
        )
    return manifest, binary


def shard_ids(inventory, split: str) -> list[int]:
    ids = sorted({key[1] for key in inventory if key[0] == split})
    for shard_id in ids:
        if (split, shard_id, "bin") not in inventory or (split, shard_id, "json") not in inventory:
            raise ValueError(f"{split} shard {shard_id:06d} is missing its pair")
    return ids


def record_stats(data: bytes):
    if len(data) % ENTRY_BYTES:
        raise ValueError("record data is not a multiple of the 72-byte ABI")
    stats = {
        "entries": len(data) // ENTRY_BYTES,
        "draws": 0,
        "mate_scores": 0,
        "absolute_score_sum": 0,
        "decisive_agreement": 0,
        "decisive_count": 0,
        "opening_ids": set(),
    }
    for offset in range(0, len(data), ENTRY_BYTES):
        record = data[offset : offset + ENTRY_BYTES]
        score = int.from_bytes(record[64:66], "little", signed=True)
        result = int.from_bytes(record[70:71], "little", signed=True)
        if result == 0:
            stats["draws"] += 1
        else:
            stats["decisive_count"] += 1
            if (score > 0 and result > 0) or (score < 0 and result < 0):
                stats["decisive_agreement"] += 1
        stats["mate_scores"] += abs(score) >= 29000
        stats["absolute_score_sum"] += abs(score)
    return stats


def merge_stats(items):
    result = {
        "entries": 0,
        "draws": 0,
        "mate_scores": 0,
        "absolute_score_sum": 0,
        "decisive_agreement": 0,
        "decisive_count": 0,
    }
    for item in items:
        for key in result:
            result[key] += item[key]
    count = result["entries"]
    decisive = result["decisive_count"]
    result["draw_rate"] = result["draws"] / count if count else 0.0
    result["mate_score_rate"] = result["mate_scores"] / count if count else 0.0
    result["mean_absolute_score"] = result["absolute_score_sum"] / count if count else 0.0
    result["score_result_agreement"] = (
        result["decisive_agreement"] / decisive if decisive else 0.0
    )
    return result


def choose_validation_ids(train_ids: list[int], split_seed: int, count: int) -> list[int]:
    ranked = sorted(
        train_ids,
        key=lambda shard_id: hashlib.sha256(
            f"phase7.1-id-validation-v1:{split_seed}:{shard_id:06d}".encode()
        ).hexdigest(),
    )
    return sorted(ranked[:count])


def fisher_yates(records: list[bytes], seed: int):
    rng = random.Random(seed)
    for index in range(len(records) - 1, 0, -1):
        other = rng.randrange(index + 1)
        records[index], records[other] = records[other], records[index]


def coprime_step(seed: int, count: int) -> tuple[int, int]:
    if count <= 1:
        return 0, 1
    offset = splitmix64(seed ^ 0x6F66_6673_6574_7631) % count
    step = max(1, splitmix64(seed ^ 0x7374_6570_2D76_3100) % count)
    while __import__("math").gcd(step, count) != 1:
        step = (step + 1) % count or 1
    return offset, step


def assemble_file(inventory, split: str, ids: list[int], output: Path, shuffle_seed: int, chunk_records: int):
    output.parent.mkdir(parents=True, exist_ok=True)
    stats = []
    with tempfile.TemporaryDirectory(prefix=f"phase7.1-{split}-") as temporary:
        temp = Path(temporary)
        chunks = []
        records = []
        chunk_index = 0
        for shard_id in ids:
            manifest, binary = load_shard(inventory, split, shard_id)
            stats.append(record_stats(binary))
            for offset in range(0, len(binary), ENTRY_BYTES):
                records.append(binary[offset : offset + ENTRY_BYTES])
                if len(records) == chunk_records:
                    fisher_yates(records, splitmix64(shuffle_seed ^ chunk_index))
                    chunk = temp / f"chunk-{chunk_index:08d}.bin"
                    chunk.write_bytes(b"".join(records))
                    chunks.append(chunk)
                    records.clear()
                    chunk_index += 1
        if records:
            fisher_yates(records, splitmix64(shuffle_seed ^ chunk_index))
            chunk = temp / f"chunk-{chunk_index:08d}.bin"
            chunk.write_bytes(b"".join(records))
            chunks.append(chunk)
        offset, step = coprime_step(shuffle_seed, len(chunks))
        with output.open("wb") as destination:
            for position in range(len(chunks)):
                with chunks[(offset + position * step) % len(chunks)].open("rb") as source:
                    shutil.copyfileobj(source, destination)
    return merge_stats(stats)


def write_json(path: Path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def verify_archives(archives: list[Path], output: Path, shard_root: Path | None = None):
    inventory, ignored, archive_hashes, duplicates = archive_inventory(archives)
    archive_inventory_count = len(inventory)
    local_count = 0
    partial_candidates = []
    if shard_root is not None:
        local, partial_candidates = directory_inventory(shard_root)
        nonempty_partials = [item for item in partial_candidates if item["bytes"]]
        if nonempty_partials:
            raise ValueError(f"non-empty partial shard artifacts found: {nonempty_partials}")
        local_count = len(local)
        for key, source in local.items():
            add_inventory_entry(inventory, key, source, duplicates)
            # The archive copy was compared by digest above. Use the local
            # copy after that explicit reconciliation so later validation and
            # assembly do not repeatedly decompress the archive.
            inventory[key] = source
    counts = {split: len(shard_ids(inventory, split)) for split in EXPECTED}
    report = {
        "schema": "haitaka-phase7.1-shard-preflight-v2",
        "archives": archive_hashes,
        "archive_inventory_entries": archive_inventory_count,
        "local_shard_root": str(shard_root) if shard_root is not None else None,
        "local_inventory_entries": local_count,
        "ignored_empty_partial_candidates": partial_candidates,
        "identical_duplicate_entries": len(duplicates),
        "local_source_preferred_after_exact_reconciliation": shard_root is not None,
        "ignored_appledouble_or_xattr_members": len(ignored),
        "valid_pair_counts": counts,
        "expected_pair_counts": EXPECTED,
        "config_hash": CONFIG_HASH,
        "passed": counts == EXPECTED,
    }
    revisions = set()
    for split in EXPECTED:
        for shard_id in shard_ids(inventory, split):
            manifest, _ = load_shard(inventory, split, shard_id)
            revisions.add(manifest.get("engine_revision"))
    report["engine_revisions"] = sorted(revisions, key=lambda value: value or "")
    report["engine_revision_gate"] = revisions.issubset({None, EXPECTED_ENGINE_REVISION})
    report["passed"] = report["passed"] and report["engine_revision_gate"]
    write_json(output, report)
    if counts != EXPECTED or not report["engine_revision_gate"]:
        raise SystemExit(
            f"shard gate failed: found train={counts['train']} and "
            f"validation={counts['validation']} shard pairs; expected "
            f"train={EXPECTED['train']} and validation={EXPECTED['validation']}; "
            f"engine revisions={report['engine_revisions']}. "
            f"Report: {output}"
        )
    return inventory


def assemble(args):
    output = args.output_root
    preflight = output / "archive-preflight.json"
    inventory = verify_archives(args.archives, preflight, args.shard_root)
    train_ids = shard_ids(inventory, "train")
    validation_ids = shard_ids(inventory, "validation")
    id_ids = choose_validation_ids(train_ids, args.split_seed, 250)
    train_split_ids = [shard_id for shard_id in train_ids if shard_id not in set(id_ids)]
    if len(train_split_ids) != 2250:
        raise ValueError("ID split did not leave exactly 2,250 train shards")

    datasets = output / "datasets"
    datasets.mkdir(parents=True, exist_ok=True)
    train_stats = assemble_file(inventory, "train", train_split_ids, datasets / "train.bin", args.shuffle_seed, args.chunk_records)
    id_stats = assemble_file(inventory, "train", id_ids, datasets / "id-validation.bin", args.shuffle_seed + 1, args.chunk_records)
    legacy_manifest, legacy_bytes = load_shard(inventory, "validation", validation_ids[0])
    del legacy_manifest, legacy_bytes
    with (datasets / "legacy-ood-validation.bin").open("wb") as destination:
        for shard_id in validation_ids:
            _, binary = load_shard(inventory, "validation", shard_id)
            destination.write(binary)

    train_openings = set()
    id_openings = set()
    games = {"train": [], "id-validation": []}
    for name, ids, target in [("train", train_split_ids, train_openings), ("id-validation", id_ids, id_openings)]:
        for shard_id in ids:
            manifest, _ = load_shard(inventory, "train", shard_id)
            target.update(manifest.get("opening_ids", []))
            games[name].extend(manifest.get("games", []))
    if not EXPECTED_OPENINGS.issubset(train_openings) or not EXPECTED_OPENINGS.issubset(id_openings):
        raise SystemExit("ID split gate failed: not all ten Phase 7 train openings are represented in both files")

    validation_stats = merge_stats(record_stats(load_shard(inventory, "validation", shard_id)[1]) for shard_id in validation_ids)
    gates = {
        "draw_rate_difference_at_most_5pp": abs(train_stats["draw_rate"] - id_stats["draw_rate"]) <= 0.05,
        "mate_score_rate_relative_difference_at_most_20pct": abs(train_stats["mate_score_rate"] - id_stats["mate_score_rate"]) <= max(train_stats["mate_score_rate"], 1e-12) * 0.20,
        "mean_absolute_score_relative_difference_at_most_20pct": abs(train_stats["mean_absolute_score"] - id_stats["mean_absolute_score"]) <= max(train_stats["mean_absolute_score"], 1e-12) * 0.20,
        "score_result_agreement_difference_at_most_5pp": abs(train_stats["score_result_agreement"] - id_stats["score_result_agreement"]) <= 0.05,
        "all_ten_train_openings_in_both": True,
        "zero_shard_overlap": set(train_split_ids).isdisjoint(id_ids),
    }
    if not all(gates.values()):
        raise SystemExit(f"ID distribution gate failed: {gates}")

    write_json(datasets / "selected-id-validation-shards.json", {
        "schema": "haitaka-phase7.1-id-shard-selection-v1",
        "split_seed": args.split_seed,
        "shuffle_seed": args.shuffle_seed,
        "shuffle_chunk_records": args.chunk_records,
        "selection_rule": "lowest-sha256-phase7.1-id-validation-v1",
        "selected_shard_ids": id_ids,
        "train_shard_ids": train_split_ids,
    })
    (datasets / "selected-id-validation-shards.txt").write_text(
        "\n".join(f"{shard_id:06d}" for shard_id in id_ids) + "\n", encoding="utf-8"
    )
    common = {
        "config_hash": CONFIG_HASH,
        "entry_bytes": ENTRY_BYTES,
        "split_seed": args.split_seed,
        "shuffle_policy": "bounded-chunk-v1-python",
        "shuffle_seed": args.shuffle_seed,
        "shuffle_chunk_records": args.chunk_records,
        "teacher_move_encoding": "unavailable",
    }
    write_json(datasets / "train.json", {**common, "dataset": "train", "game_count": len(games["train"]), "completed_games": len(games["train"]), "sampled_positions": train_stats["entries"], "opening_ids": sorted(train_openings), "games": games["train"]})
    write_json(datasets / "id-validation.json", {**common, "dataset": "id-validation", "game_count": len(games["id-validation"]), "completed_games": len(games["id-validation"]), "sampled_positions": id_stats["entries"], "opening_ids": sorted(id_openings), "games": games["id-validation"]})
    write_json(datasets / "train.audit.json", train_stats)
    write_json(datasets / "id-validation.audit.json", id_stats)
    write_json(datasets / "legacy-ood-validation.audit.json", validation_stats)
    write_json(
        datasets / "sha256.json",
        {
            name: sha256_bytes((datasets / name).read_bytes())
            for name in [
                "train.bin",
                "id-validation.bin",
                "legacy-ood-validation.bin",
            ]
        },
    )
    write_json(output / "split-gates.json", gates)
    for lane in "a", "b", "c", "d":
        lane_datasets = output / f"lane-{lane}" / "datasets"
        lane_datasets.mkdir(parents=True, exist_ok=True)
        for source_name, target_name in [
            ("train.bin", "train.bin"),
            ("train.json", "train.json"),
            ("id-validation.bin", "validation.bin"),
            ("id-validation.json", "validation.json"),
            ("legacy-ood-validation.bin", "legacy-ood-validation.bin"),
        ]:
            target = lane_datasets / target_name
            if target.exists():
                target.unlink()
            target.hardlink_to(datasets / source_name)
    print(json.dumps({"train": train_stats, "id_validation": id_stats, "legacy_ood": validation_stats, "gates": gates}, indent=2, sort_keys=True))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--archive", action="append", type=Path, dest="archives", required=True)
    parser.add_argument("--shard-root", type=Path)
    parser.add_argument("--output-root", type=Path, required=True)
    parser.add_argument("--split-seed", type=int, default=7101)
    parser.add_argument("--shuffle-seed", type=int, default=7102)
    parser.add_argument("--chunk-records", type=int, default=65536)
    parser.add_argument("--verify-only", action="store_true")
    args = parser.parse_args()
    if args.chunk_records <= 0:
        raise SystemExit("--chunk-records must be positive")
    if args.verify_only:
        verify_archives(args.archives, args.output_root / "archive-preflight.json", args.shard_root)
    else:
        assemble(args)


if __name__ == "__main__":
    main()
