#!/usr/bin/env python3
"""Resume Model B with a user-authorized renewal of the frozen rental envelope."""

from __future__ import annotations

import copy
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import sys
from typing import Any

import strength_first_model_b_trainer as model_b


base = model_b.base
ROOT = Path(__file__).resolve().parents[1]
RENEWAL_SCHEMA = "haitaka-anhoku-strength-first-approval-renewal-v1"
REPLACEMENT_OFFER_ID = 50121302
REPLACEMENT_GPU = "RTX 3090"
REPLACEMENT_COMBINED_HOURLY_RATE_USD = 0.1611111111111111
FINAL_REPLACEMENT_OFFER_ID = 45603753
FINAL_REPLACEMENT_GPU = "RTX 3090"
FINAL_REPLACEMENT_COMBINED_HOURLY_RATE_USD = 0.21111111111111108


def renewable_approval(
    path: Path,
    config: dict[str, Any],
    config_path: Path,
    *,
    now: datetime | None = None,
) -> dict[str, Any]:
    """Validate a renewed approval against the original exact cost envelope.

    The original quote timestamp establishes the offer identity and price. A
    A later explicit approval may renew time. The unavailable original offer
    may also be replaced by the single user-authorized offer pinned below;
    provider, GPU, disk, hourly rate, and budget limits remain exact.
    """

    receipt = base.load_json(path)
    base.exact_keys(
        receipt,
        {
            "schema",
            "experimentId",
            "configSha256",
            "gpuRentalApproved",
            "action",
            "approvedAt",
            "validUntil",
            "commercialEnvelope",
        },
        "GPU approval receipt",
    )
    base.exact(receipt["schema"], base.APPROVAL_SCHEMA, "approval.schema")
    base.exact(receipt["experimentId"], config["experimentId"], "approval.experimentId")
    base.exact(receipt["configSha256"], base.config_sha256(config_path), "approval.configSha256")
    base.exact(receipt["gpuRentalApproved"], True, "approval.gpuRentalApproved")
    base.exact(receipt["action"], config["authorization"]["approvedAction"], "approval.action")

    approved_at = base.timestamp(receipt["approvedAt"], "approval.approvedAt")
    valid_until = base.timestamp(receipt["validUntil"], "approval.validUntil")
    quote = config["costForecast"]["liveQuote"]
    quote_at = base.timestamp(quote["observedAt"], "costForecast.liveQuote.observedAt")
    if approved_at < quote_at or valid_until <= approved_at:
        base.fail("renewed approval has an invalid time interval")
    maximum_hours = float(config["budget"]["firstRentalEnvelopeGpuHours"])
    if (valid_until - approved_at).total_seconds() > maximum_hours * 3600.0 + 1e-6:
        base.fail("renewed approval exceeds the frozen uptime envelope")
    current_time = datetime.now(timezone.utc) if now is None else now.astimezone(timezone.utc)
    if current_time > valid_until:
        base.fail("GPU approval receipt has expired")

    envelope = base.exact_keys(
        receipt["commercialEnvelope"],
        {
            "provider",
            "offerId",
            "gpu",
            "diskGigabytes",
            "quotedCombinedHourlyRateUsd",
            "maximumCombinedHourlyRateUsd",
            "maximumRentedUptimeHours",
            "maximumTotalCostUsd",
        },
        "approval.commercialEnvelope",
    )
    expected = {
        "provider": quote["provider"],
        "offerId": envelope["offerId"],
        "gpu": envelope["gpu"],
        "diskGigabytes": quote["diskGigabytes"],
        "quotedCombinedHourlyRateUsd": envelope["quotedCombinedHourlyRateUsd"],
        "maximumCombinedHourlyRateUsd": config["costForecast"]["firstRun"]["maximumCombinedHourlyRateUsd"],
        "maximumRentedUptimeHours": config["budget"]["firstRentalEnvelopeGpuHours"],
        "maximumTotalCostUsd": config["budget"]["firstRentalEnvelopeUsd"],
    }
    base.exact(envelope, expected, "approval.commercialEnvelope")
    offer_identity = (
        envelope["offerId"],
        envelope["gpu"],
        envelope["quotedCombinedHourlyRateUsd"],
    )
    allowed_identities = {
        (quote["offerId"], quote["gpu"], quote["combinedHourlyRateUsd"]),
        (REPLACEMENT_OFFER_ID, REPLACEMENT_GPU, REPLACEMENT_COMBINED_HOURLY_RATE_USD),
        (
            FINAL_REPLACEMENT_OFFER_ID,
            FINAL_REPLACEMENT_GPU,
            FINAL_REPLACEMENT_COMBINED_HOURLY_RATE_USD,
        ),
    }
    if offer_identity not in allowed_identities:
        base.fail("approval does not bind the original or exact authorized replacement offer")
    return {
        "path": str(path),
        "sha256": base.sha256(path),
        "approvedAt": receipt["approvedAt"],
        "validUntil": receipt["validUntil"],
        "action": receipt["action"],
        "configSha256": receipt["configSha256"],
        "commercialEnvelope": envelope,
    }


def script_sha256() -> str:
    return hashlib.sha256(Path(__file__).read_bytes()).hexdigest()


def rebind_resume_state(config: dict[str, Any], config_path: Path, source: Path) -> None:
    import torch  # type: ignore

    output = base.workspace_path(config["output"]["directory"])
    state_path = output / "last-state.pt"
    if not state_path.is_file():
        base.fail("approval renewal requires an existing resume state")

    renewable_approval(source, config, config_path)
    local = base.snapshot_receipt_graph(source, output / "receipts")
    approval = renewable_approval(local, config, config_path)
    state = torch.load(state_path, map_location="cpu", weights_only=False)
    base.exact(state["configSha256"], base.config_sha256(config_path), "renewal state config SHA-256")
    if state["activeLeg"] is not None:
        base.fail("approval renewal is only allowed at a stopped gate boundary")

    old_approval = copy.deepcopy(state["approval"])
    old_path = Path(old_approval["path"])
    if not old_path.is_file() or base.sha256(old_path) != old_approval["sha256"]:
        base.fail("embedded approval receipt identity differs before renewal")
    # A later gated leg can reuse the already-bound renewal. The supplied
    # receipt was validated above and the embedded file identity matches it,
    # so no state migration or legacy quote-window validation is needed.
    if old_approval["sha256"] == approval["sha256"]:
        return
    old_receipt = base.load_json(old_path)
    # The embedded receipt can be an earlier renewal. Validate its exact
    # commercial envelope at its own approval time before replacing it.
    renewable_approval(
        old_path,
        config,
        config_path,
        now=base.timestamp(old_receipt["approvedAt"], "old approval approvedAt"),
    )

    old_state_sha = base.sha256(state_path)
    state["approval"] = approval
    base.atomic_torch_save(torch, state, state_path)
    new_state_sha = base.sha256(state_path)
    migration = {
        "schema": RENEWAL_SCHEMA,
        "experimentId": config["experimentId"],
        "configSha256": base.config_sha256(config_path),
        "optimizerUpdates": state["optimizerUpdates"],
        "authorization": "user-requested-four-more-hours",
        "renewalRunner": {"path": base.display_path(Path(__file__)), "sha256": script_sha256()},
        "oldStateSha256": old_state_sha,
        "newStateSha256": new_state_sha,
        "oldApproval": old_approval,
        "newApproval": approval,
        "changedStateFields": ["approval"],
    }
    receipt_path = output / f"approval-renewal-u{state['optimizerUpdates']:08d}.json"
    base.atomic_write_json(receipt_path, migration)
    print(f"renewed approval in {state_path}; receipt={receipt_path}")


def extract_renewal_argument(arguments: list[str]) -> tuple[Path, list[str]]:
    try:
        index = arguments.index("--renewal-approval")
        value = Path(arguments[index + 1]).resolve()
    except (ValueError, IndexError):
        base.fail("--renewal-approval PATH is required")
    return value, arguments[:index] + arguments[index + 2 :]


def option_value(arguments: list[str], name: str) -> str | None:
    try:
        return arguments[arguments.index(name) + 1]
    except (ValueError, IndexError):
        return None


def main() -> int:
    if hashlib.sha256(Path(base.__file__).read_bytes()).hexdigest() != model_b.BASE_RUNNER_SHA256:
        base.fail(f"base trainer identity differs: {base.__file__}")
    renewal, forwarded = extract_renewal_argument(sys.argv[1:])
    config_value = option_value(forwarded, "--config")
    config_path = Path(config_value).resolve() if config_value else ROOT / "r0/anhoku-strength-first/step9-model-b-training-experiment-v1.json"
    config = base.load_json(config_path)
    base.validate_config(config)
    base.validate_approval = renewable_approval
    if "train" in forwarded and "--resume" in forwarded:
        rebind_resume_state(config, config_path, renewal)
    sys.argv = [sys.argv[0], *forwarded]
    return base.main()


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except base.StrengthFirstError as exc:
        print(f"strength-first Model B renewal FAILED: {exc}", file=sys.stderr)
        raise SystemExit(1)
