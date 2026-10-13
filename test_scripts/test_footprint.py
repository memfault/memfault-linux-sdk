#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import json
import os
import pathlib
import re
import time

import pytest

from .qemu import QEMU

SAMPLE_COUNT = 5
ROLLUP_FIELDS = (
    "Rss",
    "Pss",
    "Shared_Clean",
    "Shared_Dirty",
    "Private_Clean",
    "Private_Dirty",
    "Swap",
)
STATUS_FIELDS = ("VmSize", "VmRSS", "VmHWM", "Threads")
OUTPUT_PATH = (
    pathlib.Path(__file__).parent.parent
    / "footprint"
    / os.environ.get("MACHINE", "local")
    / "runtime.json"
)


def _fields(text: str) -> dict[str, int]:
    return {m[1]: int(m[2]) for m in re.finditer(r"^(\w+):\s+(\d+)", text, re.MULTILINE)}


def _samples(qemu: QEMU, pid: str) -> list[dict[str, int]]:
    samples: list[dict[str, int]] = []
    for _ in range(SAMPLE_COUNT):
        rollup = _fields(qemu.exec_cmd_output(f"cat /proc/{pid}/smaps_rollup"))
        status = _fields(qemu.exec_cmd_output(f"cat /proc/{pid}/status"))
        samples.append({
            **{field: rollup[field] for field in ROLLUP_FIELDS},
            "Uss": rollup["Private_Clean"] + rollup["Private_Dirty"],
            **{field: status[field] for field in STATUS_FIELDS},
        })
        time.sleep(1)
    return samples


def test_footprint(qemu: QEMU) -> None:
    if not OUTPUT_PATH.parent.is_dir():
        pytest.skip(
            f"{OUTPUT_PATH.parent} does not exist; CI creates it only for the footprint cell"
        )
    qemu.systemd_wait_for_service_state("memfaultd", "active")
    pid = qemu.exec_cmd_output("systemctl show --property MainPID --value memfaultd").strip()
    started = _samples(qemu, pid)
    qemu.exec_cmd_output("memfaultctl sync", timeout=120)
    synced = _samples(qemu, pid)

    page_size = _fields(qemu.exec_cmd_output(f"grep -m1 KernelPageSize /proc/{pid}/smaps"))
    cpus = qemu.exec_cmd_output("grep -c ^processor /proc/cpuinfo").strip()
    environ = qemu.exec_cmd_output(f"tr '\\0' '\\n' < /proc/{pid}/environ")
    arena = re.search(r"^MALLOC_ARENA_MAX=(\S+)", environ, re.MULTILINE)

    OUTPUT_PATH.write_text(
        json.dumps(
            {
                "page_size_kb": page_size["KernelPageSize"],
                "cpus": int(cpus),
                "malloc_arena_max": arena[1] if arena else None,
                "samples": {"started": started, "synced": synced},
            },
            indent=2,
        )
    )
