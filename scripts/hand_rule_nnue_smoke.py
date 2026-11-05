#!/usr/bin/env python3
"""Train and export a disposable CPU NNUE using hand-rule test fixtures.

Run the Rust hand_rules_training_data_roundtrip test for each supported rule
with HAITAKA_HAND_SMOKE_OUTPUT pointing to --data first. No GPU or production
artifacts are used. Requires the existing pinned trainer and its Python venv.
"""
from __future__ import annotations

import argparse
import contextlib
import hashlib
import json
import os
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    data = args.data.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    fixtures = sorted(data.glob("standard+*.bin"))
    expected = {"nakamaware", "fushiyo", "share", "babanuki", "tensuu", "suika"}
    if {path.stem.removeprefix("standard+") for path in fixtures} != expected:
        raise RuntimeError("expected exactly the six supported single-rule fixture files")

    os.environ["CUDA_VISIBLE_DEVICES"] = ""
    os.environ["PYTHONDONTWRITEBYTECODE"] = "1"
    import r5_trainer as r5
    import strength_first_trainer as cpu

    temporary, context = r5.prepare_context({"l1": 512, "l2": 16, "l3": 32}, "cmake")
    try:
        torch, features, model_module, loader = cpu.import_cpu_fixture_trainer(context)
        torch.set_num_threads(1)
        torch.manual_seed(33)
        if torch.cuda.is_available():
            raise RuntimeError("CPU smoke must not use a GPU")
        cpu.install_cpu_feature_transformer(model_module, torch)
        cpu.install_cpu_batch_tensors(loader, torch)
        model = model_module.NNUE(features.get_feature_set_from_name("HalfKAv2^"))
        optimizer = torch.optim.SGD(model.parameters(), lr=1e-5)
        reports = []

        for fixture in fixtures:
            size = fixture.stat().st_size
            if not size or size % 72:
                raise RuntimeError(f"invalid record size in {fixture}")
            rows = size // 72
            seen, losses, active = 0, [], []
            dataset = loader.SparseBatchDataset(
                "HalfKAv2^", str(fixture), 4, cyclic=False, num_workers=1,
                filtered=False, random_fen_skipping=0, device="cpu",
            )
            for batch in dataset:
                us, them, wi, wv, bi, bv, outcome, score, psqt, stack = batch
                seen += len(us)
                active.extend((wi >= 0).sum(dim=1).tolist())
                prediction = model(us, them, wi, wv, bi, bv, psqt, stack)
                loss = ((prediction - score / 600.0) ** 2).mean()
                if not torch.isfinite(loss):
                    raise RuntimeError("nonfinite training loss")
                optimizer.zero_grad()
                loss.backward()
                optimizer.step()
                losses.append(float(loss.detach()))
            if seen != rows:
                raise RuntimeError("loader record count mismatch")
            if fixture.stem == "standard+share" and max(active) != 156:
                raise RuntimeError("maximum shared hand fixture was not exercised")
            reports.append({
                "profile": fixture.stem,
                "records": seen,
                "optimizerSteps": len(losses),
                "losses": losses,
                "maxActiveFactorizedRows": max(active),
            })

        torch.save(model, output / "smoke.pt")
        import serialize

        with (output / "serializer.log").open("w") as stream:
            with contextlib.redirect_stdout(stream):
                writer = serialize.NNUEWriter(
                    model, description="Disposable hand-rule CPU integration smoke",
                )
        network = output / "smoke.nnue"
        network.write_bytes(writer.buf)
        report = {
            "cpuOnly": True,
            "featureFamily": "HalfKAv2^",
            "rules": reports,
            "network": str(network),
            "networkSha256": hashlib.sha256(writer.buf).hexdigest(),
            "networkBytes": len(writer.buf),
        }
        (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps(report, indent=2))
    finally:
        os.chdir(Path(__file__).resolve().parents[1])
        temporary.cleanup()


if __name__ == "__main__":
    main()
