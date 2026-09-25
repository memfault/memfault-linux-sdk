#
# Copyright (c) Memfault, Inc.
# See License.txt for details
"""A zchunk delta OTA update, end to end, against the mock hawkBit DDI server.

Requires the two builds and the disk-image copy the README describes.
"""

import itertools
from collections.abc import Callable

from scripts.delta_ota_local_server import ArtifactSummary, Summary
from test_scripts.qemu import QEMU

from . import artifacts
from .artifacts import InstalledSlot

_MAX_SHARE_OF_FULL_RELEASE = 0.5
_MAX_SHARE_OF_ZCK = 0.25
_SPANS_PER_LINE = 6

Measure = Callable[[str], None]


def _artifact(summary: Summary, suffix: str) -> tuple[str, ArtifactSummary]:
    matches = {name: a for name, a in summary["artifacts"].items() if name.endswith(suffix)}
    assert len(matches) == 1, f"expected one {suffix} artifact, got {sorted(matches)}"
    return next(iter(matches.items()))


def _statuses(artifact: ArtifactSummary) -> str:
    return " ".join(f"{k}={v}" for k, v in sorted(artifact["statuses"].items()))


def _format_spans(spans: list[tuple[int, int]]) -> str:
    text = [f"{start}-{end}" for start, end in spans]
    return "\n".join(
        "  " + " ".join(text[i : i + _SPANS_PER_LINE]) for i in range(0, len(text), _SPANS_PER_LINE)
    )


def test_zck_is_fetched_only_with_range_requests(
    installed_slot: InstalledSlot, measure: Measure
) -> None:
    name, zck = _artifact(installed_slot.summary, ".zck")
    measure(f"{name}: {zck['requests']} requests [{_statuses(zck)}]")
    assert zck["statuses"].get("GET 206", 0) > 0, f"{name} was never fetched with a range request"
    # A 200 on the .zck is the whole file: either the handler fell back to a
    # full download, or a multi-range request was answered with the whole file.
    assert set(zck["statuses"]) == {"GET 206"}, f"{name} was served {zck['statuses']}"


def test_adjacent_chunk_runs_were_coalesced_into_one_range(
    installed_slot: InstalledSlot, measure: Measure
) -> None:
    name, zck = _artifact(installed_slot.summary, ".zck")
    spans = zck["request_ranges"]
    print(f"{name} was served {len(spans)} ranges:\n{_format_spans(spans)}")  # noqa: T201
    # max-ranges is 1 in sw-description-delta.in, so one range per request. A
    # range starting one byte after the previous one ends is one run of chunks
    # asked for twice, which is what the unpatched missing-range loop does.
    unmerged = [(a, b) for a, b in itertools.pairwise(spans) if a[1] + 1 == b[0]]
    measure(f"{len(spans)} ranges served, {len(unmerged)} of them adjacent to the previous")
    assert len(spans) > 1, f"{name} was served {len(spans)} range(s), nothing to coalesce"
    assert not unmerged, f"{name} was served {len(unmerged)} adjacent range pair(s):\n" + "\n".join(
        f"  {a[0]}-{a[1]} then {b[0]}-{b[1]}" for a, b in unmerged
    )


def test_delta_swu_is_fetched_whole(installed_slot: InstalledSlot, measure: Measure) -> None:
    name, swu = _artifact(installed_slot.summary, ".swu")
    measure(f"{name}: {swu['bytes']}B of {swu['size']}B [{_statuses(swu)}]")
    # swupdate HEADs the artifact before it fetches it, and a retry fetches it
    # again, so only the 200s carry a body and each one is the whole file.
    assert set(swu["statuses"]) <= {"HEAD 200", "GET 200"}, f"{name} was served {swu['statuses']}"
    gets = swu["statuses"].get("GET 200", 0)
    assert gets, f"{name} was never fetched: {swu['statuses']}"
    assert swu["size"]
    assert swu["bytes"] == swu["size"] * gets


def test_transfer_is_smaller_than_the_full_release(
    installed_slot: InstalledSlot, delta_artifacts: artifacts.DeltaArtifacts, measure: Measure
) -> None:
    transferred = sum(a["bytes"] for a in installed_slot.summary["artifacts"].values())
    full_release = delta_artifacts.full_swu_size
    share = transferred / full_release
    measure(f"{transferred}B transferred, {share:.1%} of the {full_release}B full release")
    assert transferred < full_release
    assert share < _MAX_SHARE_OF_FULL_RELEASE

    name, zck = _artifact(installed_slot.summary, ".zck")
    zck_size = zck["size"]
    assert zck_size
    zck_share = zck["bytes"] / zck_size
    measure(f"{name}: {zck['bytes']}B fetched, {zck_share:.1%} of its {zck_size}B")
    assert zck_share < _MAX_SHARE_OF_ZCK


def test_deployment_offers_the_target_version(
    installed_slot: InstalledSlot, target_software_version: str, measure: Measure
) -> None:
    version = installed_slot.summary["version"]
    measure(f"deployment version {version}, expected {target_software_version}")
    assert version == target_software_version, (
        f"the delta .swu the mock served carries version {version!r}, not the expected "
        f"{target_software_version!r}"
    )


def test_installed_partition_matches_the_release(
    installed_slot: InstalledSlot, delta_artifacts: artifacts.DeltaArtifacts, measure: Measure
) -> None:
    measure(
        f"partition {artifacts.TARGET_PARTITION} sha256 {installed_slot.partition_sha256[:16]}, "
        f"{delta_artifacts.rootfs_size}B ext4 {delta_artifacts.rootfs_sha256[:16]}"
    )
    assert installed_slot.partition_sha256 == delta_artifacts.rootfs_sha256, (
        f"partition {artifacts.TARGET_PARTITION} does not match the "
        f"{delta_artifacts.rootfs_size}B ext4 image the release was built from"
    )


def test_new_slot_booted_from_the_updated_partition(new_slot_qemu: QEMU, measure: Measure) -> None:
    measure(f"booted with root=/dev/vda{artifacts.TARGET_PARTITION}")
    new_slot_qemu.exec_cmd_ok(f"grep -q root=/dev/vda{artifacts.TARGET_PARTITION} /proc/cmdline")


def test_new_slot_reports_the_new_software_version(
    new_slot_qemu: QEMU, target_software_version: str, measure: Measure
) -> None:
    measure(f"/etc/memfaultd.conf software_version {target_software_version}")
    new_slot_qemu.exec_cmd_ok(
        f'grep -q \'"software_version": "{target_software_version}"\' /etc/memfaultd.conf',
    )


def test_new_slot_contains_the_added_package(new_slot_qemu: QEMU, measure: Measure) -> None:
    measure("strace is installed in the new slot")
    new_slot_qemu.exec_cmd_ok("command -v strace")
